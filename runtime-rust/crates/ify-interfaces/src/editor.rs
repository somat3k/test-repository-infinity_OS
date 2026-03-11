//! Stable **Editor Integration API** — interpreter attach, LSP, and runtimes.
//!
//! This module defines the [`EditorIntegrationApi`] trait that any block
//! registry in infinityOS must satisfy.
//!
//! ## Pipeline overview
//!
//! ```text
//!  register_block()
//!        │
//!        ▼
//!  create_editor(block_id, language)
//!        │
//!        ▼
//!  attach_interpreter(block_id, interpreter_type, config)
//!        │
//!        ▼
//!  bind_runtime(block_id) ──► RuntimeHandle
//! ```
//!
//! ## Stability guarantee
//!
//! The trait is versioned at
//! [`EDITOR_INTEGRATION_API_VERSION`](super::versioning::EDITOR_INTEGRATION_API_VERSION).
//!
//! ## Reference implementation
//!
//! [`BlockRegistry`](ify_controller::registry::BlockRegistry) in `ify-controller`
//! implements [`EditorIntegrationApi`].

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Common types
// ---------------------------------------------------------------------------

/// Opaque identifier for a registered block.
pub type BlockId = Uuid;

/// Summary of a live editor instance returned by [`EditorIntegrationApi`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorRef {
    /// Unique identifier for this editor session.
    pub id: Uuid,
    /// Dimension this editor belongs to.
    pub dimension_id: DimensionId,
    /// Language / MIME type (e.g. `"rust"`, `"python"`, `"typescript"`).
    pub language: String,
}

/// Summary of an interpreter attachment returned by [`EditorIntegrationApi`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterpreterRef {
    /// Unique identifier for this interpreter session.
    pub id: Uuid,
    /// Type label (e.g. `"lsp"`, `"repl"`, `"jupyter"`).
    pub interpreter_type: String,
}

/// Handle returned after successfully binding a block to the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeHandle {
    /// Unique identifier for this binding.
    pub id: Uuid,
    /// The task that owns this binding.
    pub task_id: TaskId,
    /// Executor endpoint hint (e.g. a channel address or URL).
    pub executor_endpoint: String,
}

// ---------------------------------------------------------------------------
// EditorIntegrationApi
// ---------------------------------------------------------------------------

/// Stable trait for the editor integration pipeline.
///
/// Implementors manage the three-stage pipeline that wires an editor block
/// to the interpreter and runtime: `register → create_editor →
/// attach_interpreter → bind_runtime`.
///
/// ## Semver contract
///
/// Versioned at
/// [`EDITOR_INTEGRATION_API_VERSION`](super::versioning::EDITOR_INTEGRATION_API_VERSION) `1.0.0`.
pub trait EditorIntegrationApi: Send + Sync {
    /// The error type returned by pipeline operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Register a new block within `dimension_id` and return its [`BlockId`].
    ///
    /// This is the entry-point for the pipeline.
    fn register_block(&self, dimension_id: DimensionId, task_id: TaskId) -> BlockId;

    /// Create an editor instance for `block_id` with the given `language`.
    ///
    /// Returns an [`EditorRef`] containing the new editor's identity and
    /// language configuration.
    ///
    /// ## Errors
    ///
    /// Returns [`Err`] if `block_id` does not exist, or if an editor has
    /// already been created for this block.
    fn create_editor(
        &self,
        block_id: BlockId,
        language: &str,
    ) -> Result<EditorRef, Self::Error>;

    /// Attach an interpreter to `block_id`.
    ///
    /// `interpreter_type` identifies the kind of interpreter (e.g. `"lsp"`,
    /// `"repl"`, `"jupyter"`).  `config` is passed verbatim to the interpreter
    /// initialiser.
    ///
    /// ## Errors
    ///
    /// Returns [`Err`] if `block_id` does not exist, if no editor has been
    /// created yet, or if an interpreter is already attached.
    fn attach_interpreter(
        &self,
        block_id: BlockId,
        interpreter_type: &str,
        config: serde_json::Value,
    ) -> Result<InterpreterRef, Self::Error>;

    /// Bind `block_id` to the executor runtime.
    ///
    /// Returns a [`RuntimeHandle`] that callers use to route execution
    /// requests to the correct executor endpoint.
    ///
    /// ## Errors
    ///
    /// Returns [`Err`] if `block_id` does not exist, if a required pipeline
    /// stage is incomplete, or if the block is already bound.
    fn bind_runtime(&self, block_id: BlockId) -> Result<RuntimeHandle, Self::Error>;

    /// Look up the editor associated with `block_id`, if any.
    fn editor_for(&self, block_id: BlockId) -> Option<EditorRef>;

    /// Look up the runtime binding associated with `block_id`, if any.
    fn binding_for(&self, block_id: BlockId) -> Option<RuntimeHandle>;
}
