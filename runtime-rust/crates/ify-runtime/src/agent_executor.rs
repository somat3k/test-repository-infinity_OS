//! Executor for agentic combo ML tasks.
//!
//! [`AgentExecutor`] extends the base task executor with ML-specific routing,
//! capability checks, yield-point integration, and structured span tracing.
//!
//! ## Agent task kinds
//!
//! | Kind             | Description                                                     |
//! |------------------|-----------------------------------------------------------------|
//! | `Generic`        | Plain async task with no ML routing                             |
//! | `InferenceML`    | Single-model inference (requires `INVOKE_MODEL`)                |
//! | `EmbeddingML`    | Embedding generation for vector store ingestion                 |
//! | `ComboML`        | Chained inference + tool calls (requires `INVOKE_MODEL` + tools)|
//!
//! ## Cancellation
//!
//! Each [`AgentTask`] carries a [`crate::yield_token::YieldToken`].  Callers
//! can cancel an in-flight task by calling [`crate::yield_token::YieldTokenSource::cancel`]
//! on the corresponding source.  The task's future must call
//! [`crate::yield_token::YieldToken::yield_now`] or
//! [`crate::yield_token::YieldToken::check_cancelled`] at safe points.
//!
//! ## Sandbox integration
//!
//! `AgentExecutor::submit` validates the task's required capabilities against
//! the executor's sandbox before accepting submission.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ify_core::{Capabilities, DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, instrument, warn};

use crate::sandbox::{Sandbox, SandboxPolicy};
use crate::yield_token::{YieldError, YieldToken};

// ---------------------------------------------------------------------------
// AgentTaskKind
// ---------------------------------------------------------------------------

/// Discriminant for the type of agentic ML task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentTaskKind {
    /// A plain async task with no ML routing.
    Generic,
    /// Single-model inference (requires [`ify_core::Capabilities::INVOKE_MODEL`]).
    InferenceML,
    /// Embedding generation for vector store ingestion.
    EmbeddingML,
    /// Chained inference + tool calls
    /// (requires `INVOKE_MODEL` and `INVOKE_TOOLS`).
    ComboML,
}

impl AgentTaskKind {
    /// Return the capability set required to run this task kind.
    pub fn required_caps(&self) -> Capabilities {
        match self {
            Self::Generic => Capabilities::SCHEDULER,
            Self::InferenceML | Self::EmbeddingML => {
                Capabilities::SCHEDULER | Capabilities::INVOKE_MODEL
            }
            Self::ComboML => {
                Capabilities::SCHEDULER | Capabilities::INVOKE_MODEL | Capabilities::INVOKE_TOOLS
            }
        }
    }
}

impl std::fmt::Display for AgentTaskKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generic => write!(f, "generic"),
            Self::InferenceML => write!(f, "inference-ml"),
            Self::EmbeddingML => write!(f, "embedding-ml"),
            Self::ComboML => write!(f, "combo-ml"),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the agent executor.
#[derive(Debug, Error)]
pub enum AgentExecutorError {
    /// Capability check failed before submission.
    #[error("capability denied for task kind {kind}: required {required:?}, granted {granted:?}")]
    CapabilityDenied {
        /// Task kind that was attempted.
        kind: AgentTaskKind,
        /// Capabilities required by the task kind.
        required: Capabilities,
        /// Capabilities actually granted.
        granted: Capabilities,
    },

    /// The executor's internal queue is full.
    #[error("agent executor queue is full (capacity {0})")]
    QueueFull(usize),

    /// The task was cancelled via its yield token.
    #[error("task {0} was cancelled")]
    Cancelled(TaskId),

    /// The task failed with an error message.
    #[error("task {task_id} failed: {message}")]
    TaskFailed {
        /// Task that failed.
        task_id: TaskId,
        /// Error message.
        message: String,
    },
}

impl From<YieldError> for AgentExecutorError {
    fn from(e: YieldError) -> Self {
        match e {
            YieldError::Cancelled => AgentExecutorError::Cancelled(TaskId::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// AgentTask
// ---------------------------------------------------------------------------

/// Boxed async future type for an agent task.
pub type AgentFuture =
    Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + 'static>>;

/// Descriptor for a task submitted to the [`AgentExecutor`].
pub struct AgentTask {
    /// Stable task identifier.
    pub task_id: TaskId,
    /// Dimension this task belongs to.
    pub dimension_id: DimensionId,
    /// Task kind used for capability routing.
    pub kind: AgentTaskKind,
    /// Cooperative cancellation token for the task's future.
    pub yield_token: YieldToken,
    /// Async future that performs the actual work.
    pub future: AgentFuture,
}

/// Summary returned after a task completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskResult {
    /// Task identifier.
    pub task_id: TaskId,
    /// Dimension identifier.
    pub dimension_id: DimensionId,
    /// Task kind.
    pub kind: AgentTaskKind,
    /// JSON output produced by the task (Null on failure or cancellation).
    pub output: serde_json::Value,
    /// Whether the task succeeded.
    pub success: bool,
    /// Error message if the task failed or was cancelled.
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for [`AgentExecutor`].
#[derive(Debug, Clone)]
pub struct AgentExecutorConfig {
    /// Maximum number of concurrently executing agent tasks.  Default: 16.
    pub max_concurrent: usize,
    /// Maximum pending-queue depth.  Default: 256.
    pub queue_depth: usize,
}

impl Default for AgentExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 16,
            queue_depth: 256,
        }
    }
}

// ---------------------------------------------------------------------------
// AgentExecutor
// ---------------------------------------------------------------------------

/// Executor for agentic combo ML tasks.
///
/// Wraps capability checking, sandbox entry, cooperative yield, and structured
/// tracing around async futures that perform agentic work.
pub struct AgentExecutor {
    dimension_id: DimensionId,
    config: AgentExecutorConfig,
    sandbox: Arc<Sandbox>,
    pending: tokio::sync::Mutex<Vec<AgentTask>>,
}

impl AgentExecutor {
    /// Create a new executor for the given dimension.
    ///
    /// `granted_caps` is the capability set obtained from the kernel (or a
    /// test-supplied set).
    pub fn new(
        dimension_id: DimensionId,
        config: AgentExecutorConfig,
        granted_caps: Capabilities,
    ) -> Self {
        Self {
            dimension_id,
            config,
            sandbox: Arc::new(Sandbox::new(granted_caps)),
            pending: tokio::sync::Mutex::new(Vec::new()),
        }
    }

    /// Submit an agent task for execution.
    ///
    /// Validates the task's required capabilities against the sandbox before
    /// accepting it into the queue.
    ///
    /// # Errors
    ///
    /// - [`AgentExecutorError::CapabilityDenied`] — task kind requires caps
    ///   not granted to this executor.
    /// - [`AgentExecutorError::QueueFull`] — queue is at capacity.
    #[instrument(skip(self, task), fields(
        task_id  = %task.task_id,
        kind     = %task.kind,
        dim      = %self.dimension_id,
    ))]
    pub async fn submit(&self, task: AgentTask) -> Result<TaskId, AgentExecutorError> {
        let required = task.kind.required_caps();
        let policy = SandboxPolicy {
            required,
            allowed: required,
            limits: Default::default(),
            label: format!("agent-task:{}", task.kind),
        };

        self.sandbox.enter(policy).map_err(|_| {
            warn!(
                task_id = %task.task_id,
                kind    = %task.kind,
                required = ?required,
                "capability denied: rejecting task"
            );
            AgentExecutorError::CapabilityDenied {
                kind: task.kind.clone(),
                required,
                granted: self.sandbox.granted_caps(),
            }
        })?;

        let mut pending = self.pending.lock().await;
        if pending.len() >= self.config.queue_depth {
            return Err(AgentExecutorError::QueueFull(self.config.queue_depth));
        }
        let task_id = task.task_id;
        info!(task_id = %task_id, kind = %task.kind, "agent task queued");
        pending.push(task);
        Ok(task_id)
    }

    /// Drain and execute all queued tasks, respecting `max_concurrent`.
    ///
    /// Returns one [`AgentTaskResult`] per submitted task in the order they
    /// complete (not the order they were submitted).
    pub async fn run_all(&self) -> Vec<AgentTaskResult> {
        let tasks: Vec<AgentTask> = {
            let mut pending = self.pending.lock().await;
            std::mem::take(&mut *pending)
        };

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent));
        let results = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut handles = Vec::new();

        for task in tasks {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("semaphore closed");
            let results = results.clone();

            let handle = tokio::spawn(async move {
                let task_id = task.task_id;
                let dimension_id = task.dimension_id;
                let kind = task.kind.clone();
                let token = task.yield_token;

                debug!(task_id = %task_id, kind = %kind, "agent task starting");

                // Check cancellation before starting.
                if token.check_cancelled().is_err() {
                    drop(permit);
                    let result = AgentTaskResult {
                        task_id,
                        dimension_id,
                        kind,
                        output: serde_json::Value::Null,
                        success: false,
                        error: Some("cancelled before start".to_owned()),
                    };
                    results.lock().await.push(result);
                    return;
                }

                let outcome = task.future.await;
                drop(permit);

                let result = match outcome {
                    Ok(output) => {
                        info!(task_id = %task_id, "agent task completed");
                        AgentTaskResult {
                            task_id,
                            dimension_id,
                            kind,
                            output,
                            success: true,
                            error: None,
                        }
                    }
                    Err(msg) => {
                        let is_cancel = msg.contains("cancelled");
                        if is_cancel {
                            warn!(task_id = %task_id, "agent task cancelled");
                        } else {
                            tracing::error!(task_id = %task_id, error = %msg, "agent task failed");
                        }
                        AgentTaskResult {
                            task_id,
                            dimension_id,
                            kind,
                            output: serde_json::Value::Null,
                            success: false,
                            error: Some(msg),
                        }
                    }
                };
                results.lock().await.push(result);
            });
            handles.push(handle);
        }

        for h in handles {
            let _ = h.await;
        }

        Arc::try_unwrap(results)
            .expect("all handles completed")
            .into_inner()
    }

    /// Return the dimension this executor is bound to.
    pub fn dimension_id(&self) -> DimensionId {
        self.dimension_id
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yield_token::YieldTokenSource;
    use ify_core::DimensionId;

    fn all_caps() -> Capabilities {
        Capabilities::all()
    }

    fn executor() -> AgentExecutor {
        AgentExecutor::new(DimensionId::new(), AgentExecutorConfig::default(), all_caps())
    }

    fn make_task(kind: AgentTaskKind, future: AgentFuture) -> AgentTask {
        AgentTask {
            task_id: TaskId::new(),
            dimension_id: DimensionId::new(),
            kind,
            yield_token: YieldToken::never_cancelled(),
            future,
        }
    }

    #[tokio::test]
    async fn generic_task_completes() {
        let exec = executor();
        let task = make_task(
            AgentTaskKind::Generic,
            Box::pin(async { Ok(serde_json::json!({"result": 42})) }),
        );
        exec.submit(task).await.unwrap();
        let results = exec.run_all().await;
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].output, serde_json::json!({"result": 42}));
    }

    #[tokio::test]
    async fn inference_ml_task_completes() {
        let exec = executor();
        let task = make_task(
            AgentTaskKind::InferenceML,
            Box::pin(async { Ok(serde_json::json!({"prediction": 0.9})) }),
        );
        exec.submit(task).await.unwrap();
        let results = exec.run_all().await;
        assert!(results[0].success);
    }

    #[tokio::test]
    async fn combo_ml_task_completes() {
        let exec = executor();
        let task = make_task(
            AgentTaskKind::ComboML,
            Box::pin(async { Ok(serde_json::json!({"combo": true})) }),
        );
        exec.submit(task).await.unwrap();
        let results = exec.run_all().await;
        assert!(results[0].success);
    }

    #[tokio::test]
    async fn failed_task_records_error() {
        let exec = executor();
        let task = make_task(
            AgentTaskKind::Generic,
            Box::pin(async { Err("deliberate error".to_owned()) }),
        );
        exec.submit(task).await.unwrap();
        let results = exec.run_all().await;
        assert!(!results[0].success);
        assert!(results[0].error.as_ref().unwrap().contains("deliberate"));
    }

    #[tokio::test]
    async fn capability_denied_for_inference_without_model_cap() {
        let exec = AgentExecutor::new(
            DimensionId::new(),
            AgentExecutorConfig::default(),
            Capabilities::SCHEDULER, // no INVOKE_MODEL
        );
        let task = make_task(
            AgentTaskKind::InferenceML,
            Box::pin(async { Ok(serde_json::json!(null)) }),
        );
        let err = exec.submit(task).await.unwrap_err();
        assert!(matches!(err, AgentExecutorError::CapabilityDenied { .. }));
    }

    #[tokio::test]
    async fn cancelled_task_recorded() {
        let exec = executor();
        let src = YieldTokenSource::new();
        src.cancel(); // cancel before submission
        let task = AgentTask {
            task_id: TaskId::new(),
            dimension_id: DimensionId::new(),
            kind: AgentTaskKind::Generic,
            yield_token: src.token(),
            future: Box::pin(async { Ok(serde_json::json!(null)) }),
        };
        exec.submit(task).await.unwrap();
        let results = exec.run_all().await;
        assert!(!results[0].success);
        assert!(results[0].error.as_ref().unwrap().contains("cancelled"));
    }

    #[tokio::test]
    async fn queue_full_rejected() {
        let exec = AgentExecutor::new(
            DimensionId::new(),
            AgentExecutorConfig {
                max_concurrent: 1,
                queue_depth: 1,
            },
            all_caps(),
        );
        exec.submit(make_task(
            AgentTaskKind::Generic,
            Box::pin(async { Ok(serde_json::json!(null)) }),
        ))
        .await
        .unwrap();
        let err = exec
            .submit(make_task(
                AgentTaskKind::Generic,
                Box::pin(async { Ok(serde_json::json!(null)) }),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, AgentExecutorError::QueueFull(1)));
    }

    #[test]
    fn agent_task_kind_required_caps() {
        assert!(AgentTaskKind::Generic
            .required_caps()
            .contains(Capabilities::SCHEDULER));
        assert!(AgentTaskKind::InferenceML
            .required_caps()
            .contains(Capabilities::INVOKE_MODEL));
        assert!(AgentTaskKind::ComboML
            .required_caps()
            .contains(Capabilities::INVOKE_TOOLS));
    }
}
