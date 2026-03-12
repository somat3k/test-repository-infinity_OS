//! Tool runner abstraction — typed interface over db/http/blockchain/model tools.
//!
//! The [`ToolRunner`] trait abstracts over heterogeneous back-end tools so
//! that the agent executor can invoke them uniformly.  Each tool implementation
//! is registered in a [`ToolRegistry`] and dispatched by [`ToolKind`].
//!
//! ## Tool kinds
//!
//! | Kind          | Description                                          |
//! |---------------|------------------------------------------------------|
//! | `Db`          | Relational / document database queries               |
//! | `Http`        | Outbound HTTP/REST calls                             |
//! | `Blockchain`  | On-chain reads and signed transactions               |
//! | `Model`       | ML model inference (local or remote)                 |
//! | `Custom(id)`  | Extension point for domain-specific tool types       |
//!
//! ## Sandbox integration
//!
//! Tool invocations are capability-gated: callers must present a
//! [`crate::sandbox::SandboxGuard`] with [`ify_core::Capabilities::INVOKE_TOOLS`]
//! (and [`ify_core::Capabilities::INVOKE_MODEL`] for model tools) before the
//! registry will dispatch the call.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ify_core::{Capabilities, DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, instrument, warn};

use crate::sandbox::SandboxGuard;

// ---------------------------------------------------------------------------
// Tool kind
// ---------------------------------------------------------------------------

/// Discriminant for the category of a tool.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolKind {
    /// Relational / document database.
    Db,
    /// Outbound HTTP/REST.
    Http,
    /// Blockchain RPC / wallet signing.
    Blockchain,
    /// ML model inference.
    Model,
    /// Custom extension point.
    Custom(String),
}

impl std::fmt::Display for ToolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db => write!(f, "db"),
            Self::Http => write!(f, "http"),
            Self::Blockchain => write!(f, "blockchain"),
            Self::Model => write!(f, "model"),
            Self::Custom(id) => write!(f, "custom:{id}"),
        }
    }
}

// ---------------------------------------------------------------------------
// ToolRequest / ToolResponse
// ---------------------------------------------------------------------------

/// A request to invoke a registered tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRequest {
    /// Tool identifier — must match a key in [`ToolRegistry`].
    pub tool_id: String,
    /// Kind discriminant used for capability checks and routing.
    pub kind: ToolKind,
    /// Dimension that owns this invocation.
    pub dimension_id: DimensionId,
    /// Task that issued this invocation.
    pub task_id: TaskId,
    /// JSON payload forwarded to the tool implementation.
    pub payload: serde_json::Value,
}

/// The result returned by a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    /// Echo of the originating `tool_id`.
    pub tool_id: String,
    /// JSON output produced by the tool.
    pub output: serde_json::Value,
    /// Wall-clock duration of the tool call in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// ToolRunner trait
// ---------------------------------------------------------------------------

/// Errors from tool dispatch.
#[derive(Debug, Error)]
pub enum ToolError {
    /// No tool with the given `tool_id` is registered.
    #[error("tool not found: {0}")]
    NotFound(String),

    /// The caller's sandbox does not have the required capability.
    #[error("capability denied for tool {tool_id}: requires {required:?}")]
    CapabilityDenied {
        /// Tool identifier.
        tool_id: String,
        /// Capability that was missing.
        required: Capabilities,
    },

    /// The tool returned an error.
    #[error("tool {tool_id} error: {message}")]
    ToolFailed {
        /// Tool identifier.
        tool_id: String,
        /// Error message from the tool.
        message: String,
    },

    /// Serialization/deserialization failure.
    #[error("serialisation error: {0}")]
    Serialisation(String),
}

/// Boxed async function type for a tool implementation.
type BoxToolFn = Arc<
    dyn Fn(ToolRequest) -> Pin<Box<dyn Future<Output = Result<ToolResponse, ToolError>> + Send>>
        + Send
        + Sync,
>;

/// Metadata stored alongside a tool implementation.
struct ToolEntry {
    kind: ToolKind,
    runner: BoxToolFn,
}

// ---------------------------------------------------------------------------
// ToolRegistry
// ---------------------------------------------------------------------------

/// Registry of named tool implementations.
///
/// Register tools with [`ToolRegistry::register`] and dispatch them with
/// [`ToolRegistry::run`].
pub struct ToolRegistry {
    tools: HashMap<String, ToolEntry>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool implementation.
    ///
    /// `runner` must be an `async fn(ToolRequest) -> Result<ToolResponse, ToolError>`.
    pub fn register<F, Fut>(&mut self, tool_id: impl Into<String>, kind: ToolKind, runner: F)
    where
        F: Fn(ToolRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<ToolResponse, ToolError>> + Send + 'static,
    {
        let id = tool_id.into();
        let boxed: BoxToolFn = Arc::new(move |req| Box::pin(runner(req)));
        self.tools.insert(id, ToolEntry { kind, runner: boxed });
    }

    /// Dispatch a [`ToolRequest`], checking sandbox capabilities first.
    ///
    /// # Capability requirements
    ///
    /// All tool kinds require [`Capabilities::INVOKE_TOOLS`].
    /// [`ToolKind::Model`] additionally requires [`Capabilities::INVOKE_MODEL`].
    ///
    /// # Errors
    ///
    /// - [`ToolError::NotFound`] — no tool with that id.
    /// - [`ToolError::CapabilityDenied`] — sandbox missing required capability.
    /// - [`ToolError::ToolFailed`] — tool returned an error.
    #[instrument(skip(self, guard), fields(tool_id = %request.tool_id))]
    pub async fn run(
        &self,
        guard: &SandboxGuard,
        request: ToolRequest,
    ) -> Result<ToolResponse, ToolError> {
        // All tool invocations require INVOKE_TOOLS.
        guard
            .assert_capability(Capabilities::INVOKE_TOOLS)
            .map_err(|_| ToolError::CapabilityDenied {
                tool_id: request.tool_id.clone(),
                required: Capabilities::INVOKE_TOOLS,
            })?;

        let entry = self.tools.get(&request.tool_id).ok_or_else(|| {
            warn!(tool_id = %request.tool_id, "tool not found");
            ToolError::NotFound(request.tool_id.clone())
        })?;

        // Model tools require an additional capability.
        if entry.kind == ToolKind::Model {
            guard
                .assert_capability(Capabilities::INVOKE_MODEL)
                .map_err(|_| ToolError::CapabilityDenied {
                    tool_id: request.tool_id.clone(),
                    required: Capabilities::INVOKE_MODEL,
                })?;
        }

        debug!(
            tool_id = %request.tool_id,
            kind = %entry.kind,
            "dispatching tool"
        );

        let start = std::time::Instant::now();
        let mut response = (entry.runner)(request).await?;
        response.duration_ms = start.elapsed().as_millis() as u64;
        Ok(response)
    }

    /// Return the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Return `true` if no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Convenience: ToolRunner trait for single-tool implementations
// ---------------------------------------------------------------------------

/// Trait implemented by individual tool backends.
///
/// Implement this for each tool kind; then register via
/// [`ToolRegistry::register`].
pub trait ToolRunner: Send + Sync {
    /// Run the tool with the given request.
    fn run(
        &self,
        request: ToolRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResponse, ToolError>> + Send + '_>>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{Sandbox, SandboxPolicy};
    use ify_core::{Capabilities, DimensionId, TaskId};

    fn make_guard_with(caps: Capabilities) -> SandboxGuard {
        let sb = Sandbox::new(caps);
        sb.enter(SandboxPolicy::minimal(caps)).unwrap()
    }

    fn echo_tool(req: ToolRequest) -> impl Future<Output = Result<ToolResponse, ToolError>> {
        async move {
            Ok(ToolResponse {
                tool_id: req.tool_id,
                output: req.payload,
                duration_ms: 0,
            })
        }
    }

    fn build_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register("echo-db", ToolKind::Db, echo_tool);
        reg.register("echo-http", ToolKind::Http, echo_tool);
        reg.register("echo-chain", ToolKind::Blockchain, echo_tool);
        reg.register("echo-model", ToolKind::Model, echo_tool);
        reg
    }

    fn req(tool_id: &str, kind: ToolKind) -> ToolRequest {
        ToolRequest {
            tool_id: tool_id.to_owned(),
            kind,
            dimension_id: DimensionId::new(),
            task_id: TaskId::new(),
            payload: serde_json::json!({"key": "value"}),
        }
    }

    #[tokio::test]
    async fn db_tool_dispatched() {
        let reg = build_registry();
        let guard = make_guard_with(
            Capabilities::INVOKE_TOOLS | Capabilities::INVOKE_MODEL,
        );
        let resp = reg
            .run(&guard, req("echo-db", ToolKind::Db))
            .await
            .unwrap();
        assert_eq!(resp.output, serde_json::json!({"key": "value"}));
    }

    #[tokio::test]
    async fn model_tool_requires_invoke_model() {
        let reg = build_registry();
        // Only INVOKE_TOOLS, not INVOKE_MODEL → should be denied for model tool.
        let guard = make_guard_with(Capabilities::INVOKE_TOOLS);
        let err = reg
            .run(&guard, req("echo-model", ToolKind::Model))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::CapabilityDenied { .. }));
    }

    #[tokio::test]
    async fn unknown_tool_returns_not_found() {
        let reg = build_registry();
        let guard = make_guard_with(Capabilities::INVOKE_TOOLS | Capabilities::INVOKE_MODEL);
        let err = reg
            .run(&guard, req("nonexistent", ToolKind::Http))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[tokio::test]
    async fn missing_invoke_tools_cap_denied() {
        let reg = build_registry();
        let guard = make_guard_with(Capabilities::MEMORY); // no INVOKE_TOOLS
        let err = reg
            .run(&guard, req("echo-http", ToolKind::Http))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::CapabilityDenied { .. }));
    }

    #[test]
    fn registry_len_tracks_registrations() {
        let reg = build_registry();
        assert_eq!(reg.len(), 4);
    }

    #[test]
    fn tool_kind_display() {
        assert_eq!(ToolKind::Db.to_string(), "db");
        assert_eq!(ToolKind::Http.to_string(), "http");
        assert_eq!(ToolKind::Blockchain.to_string(), "blockchain");
        assert_eq!(ToolKind::Model.to_string(), "model");
        assert_eq!(ToolKind::Custom("mylib".to_owned()).to_string(), "custom:mylib");
    }
}
