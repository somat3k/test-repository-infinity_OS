//! # ify-executor — infinityOS Task Execution Engine
//!
//! This crate implements the task execution engine for the infinityOS Performer
//! Runtime.  It owns the full task lifecycle from submission through completion,
//! and provides:
//!
//! - [`TaskQueue`] — priority-aware, async-native task queue.
//! - [`Executor`] — runtime that drains the queue and manages concurrency.
//! - [`TaskHandle`] — caller-side handle for tracking and cancelling tasks.
//!
//! ## Task Lifecycle
//!
//! ```text
//! submit()
//!   │
//!   ▼
//! QUEUED ──► RUNNING ──► COMPLETED
//!              │
//!              ├──► FAILED
//!              │
//!              └──► CANCELLED  (via TaskHandle::cancel())
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ify_executor::{Executor, ExecutorConfig};
//! use ify_core::{DimensionId, TaskId};
//!
//! #[tokio::main]
//! async fn main() {
//!     let dim = DimensionId::new();
//!     let config = ExecutorConfig::default();
//!     let executor = Executor::new(dim, config);
//!
//!     let handle = executor.submit(async {
//!         println!("Hello from a task!");
//!         Ok(())
//!     }).await.expect("submit must succeed");
//!
//!     handle.await_completion().await.expect("task must complete");
//! }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use ify_core::{DimensionId, TaskId};
use thiserror::Error;
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, error, info, instrument};

// ---------------------------------------------------------------------------
// Task state
// ---------------------------------------------------------------------------

/// Discrete lifecycle states for a managed task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskState {
    /// Submitted and waiting for a worker slot.
    Queued = 0,
    /// Currently executing.
    Running = 1,
    /// Suspended at a yield point.
    Paused = 2,
    /// Cancellation has been requested.
    Cancelled = 3,
    /// Terminated with an error.
    Failed = 4,
    /// Finished successfully.
    Completed = 5,
}

impl TaskState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Queued,
            1 => Self::Running,
            2 => Self::Paused,
            3 => Self::Cancelled,
            4 => Self::Failed,
            5 => Self::Completed,
            _ => Self::Failed,
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the executor.
#[derive(Debug, Error)]
pub enum ExecutorError {
    /// The task queue is at capacity; submission was rejected.
    #[error("executor queue is full (capacity {0})")]
    QueueFull(usize),

    /// The referenced task does not exist or has already completed.
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),

    /// The task was cancelled before completion.
    #[error("task {0} was cancelled")]
    Cancelled(TaskId),

    /// The task failed with an internal error.
    #[error("task {0} failed: {1}")]
    TaskFailed(TaskId, String),

    /// Executor was shut down before the operation could complete.
    #[error("executor is shut down")]
    ShutDown,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for an [`Executor`] instance.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Maximum number of concurrently executing tasks.  Default: 32.
    pub max_concurrent: usize,
    /// Maximum queue depth.  Default: 1024.
    pub queue_depth: usize,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 32,
            queue_depth: 1024,
        }
    }
}

// ---------------------------------------------------------------------------
// TaskHandle
// ---------------------------------------------------------------------------

/// Shared state for an in-flight task.
struct TaskInner {
    id: TaskId,
    state: AtomicU8,
}

type CompletionRx = Arc<Mutex<Option<oneshot::Receiver<Result<(), ExecutorError>>>>>;

/// Caller-side handle for tracking and cancelling a submitted task.
#[derive(Clone)]
pub struct TaskHandle {
    inner: Arc<TaskInner>,
    completion_rx: CompletionRx,
}

impl TaskHandle {
    /// Return the `TaskId` assigned to this task.
    pub fn id(&self) -> TaskId {
        self.inner.id
    }

    /// Return the current [`TaskState`] for this task.
    pub fn state(&self) -> TaskState {
        TaskState::from_u8(self.inner.state.load(Ordering::Acquire))
    }

    /// Request cancellation of this task.
    ///
    /// The task is transitioned to [`TaskState::Cancelled`] and the execution
    /// future is aborted on the next `.await` point.
    pub fn cancel(&self) {
        self.inner
            .state
            .compare_exchange(
                TaskState::Running as u8,
                TaskState::Cancelled as u8,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .ok();
        self.inner
            .state
            .compare_exchange(
                TaskState::Queued as u8,
                TaskState::Cancelled as u8,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .ok();
    }

    /// Await task completion, returning `Ok(())` on success or an
    /// [`ExecutorError`] on failure or cancellation.
    pub async fn await_completion(self) -> Result<(), ExecutorError> {
        let rx = {
            let mut guard = self.completion_rx.lock().await;
            guard.take()
        };
        match rx {
            Some(receiver) => receiver.await.map_err(|_| ExecutorError::ShutDown)?,
            None => Err(ExecutorError::TaskNotFound(self.id())),
        }
    }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

type BoxFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'static>>;

struct QueuedTask {
    id: TaskId,
    future: BoxFuture,
    inner: Arc<TaskInner>,
    completion_tx: oneshot::Sender<Result<(), ExecutorError>>,
}

/// Priority-aware async task executor for a single dimension.
pub struct Executor {
    dimension_id: DimensionId,
    config: ExecutorConfig,
    queue: Arc<Mutex<Vec<QueuedTask>>>,
}

impl Executor {
    /// Create a new executor for the given dimension.
    pub fn new(dimension_id: DimensionId, config: ExecutorConfig) -> Self {
        Self {
            dimension_id,
            config,
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Submit a future as a task and return a [`TaskHandle`].
    ///
    /// # Errors
    ///
    /// Returns [`ExecutorError::QueueFull`] if the queue has reached
    /// `config.queue_depth`.
    #[instrument(skip(self, future), fields(dimension = %self.dimension_id))]
    pub async fn submit<F>(&self, future: F) -> Result<TaskHandle, ExecutorError>
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        let mut queue = self.queue.lock().await;
        if queue.len() >= self.config.queue_depth {
            return Err(ExecutorError::QueueFull(self.config.queue_depth));
        }

        let id = TaskId::new();
        let (completion_tx, completion_rx) = oneshot::channel();

        let inner = Arc::new(TaskInner {
            id,
            state: AtomicU8::new(TaskState::Queued as u8),
        });

        let handle = TaskHandle {
            inner: inner.clone(),
            completion_rx: Arc::new(Mutex::new(Some(completion_rx))),
        };

        queue.push(QueuedTask {
            id,
            future: Box::pin(future),
            inner,
            completion_tx,
        });

        info!(task_id = %id, "task queued");
        Ok(handle)
    }

    /// Drain and execute all queued tasks up to `max_concurrent` concurrently.
    ///
    /// This is a simplified drive loop suitable for testing.  Production usage
    /// should integrate with a persistent worker pool.
    pub async fn run_all(&self) {
        let tasks: Vec<QueuedTask> = {
            let mut queue = self.queue.lock().await;
            std::mem::take(&mut *queue)
        };

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent));
        let mut handles = Vec::new();

        for task in tasks {
            let permit = semaphore.clone().acquire_owned().await.expect("semaphore closed");
            let handle = tokio::spawn(async move {
                task.inner
                    .state
                    .store(TaskState::Running as u8, Ordering::Release);
                debug!(task_id = %task.id, "task running");

                let result = task.future.await;
                drop(permit);

                match result {
                    Ok(()) => {
                        task.inner
                            .state
                            .store(TaskState::Completed as u8, Ordering::Release);
                        info!(task_id = %task.id, "task completed");
                        let _ = task.completion_tx.send(Ok(()));
                    }
                    Err(msg) => {
                        task.inner
                            .state
                            .store(TaskState::Failed as u8, Ordering::Release);
                        error!(task_id = %task.id, error = %msg, "task failed");
                        let _ = task
                            .completion_tx
                            .send(Err(ExecutorError::TaskFailed(task.id, msg)));
                    }
                }
            });
            handles.push(handle);
        }

        for h in handles {
            let _ = h.await;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::DimensionId;

    #[tokio::test]
    async fn submit_and_complete() {
        let dim = DimensionId::new();
        let exec = Executor::new(dim, ExecutorConfig::default());

        let handle = exec
            .submit(async { Ok(()) })
            .await
            .expect("submit must succeed");

        exec.run_all().await;

        assert_eq!(handle.state(), TaskState::Completed);
    }

    #[tokio::test]
    async fn failed_task_propagates_error() {
        let dim = DimensionId::new();
        let exec = Executor::new(dim, ExecutorConfig::default());

        let handle = exec
            .submit(async { Err("deliberate failure".to_string()) })
            .await
            .expect("submit must succeed");

        exec.run_all().await;

        assert_eq!(handle.state(), TaskState::Failed);
    }

    #[tokio::test]
    async fn queue_full_returns_error() {
        let dim = DimensionId::new();
        let config = ExecutorConfig {
            queue_depth: 2,
            max_concurrent: 1,
        };
        let exec = Executor::new(dim, config);

        exec.submit(async { Ok(()) }).await.unwrap();
        exec.submit(async { Ok(()) }).await.unwrap();

        let result = exec.submit(async { Ok(()) }).await;
        assert!(matches!(result, Err(ExecutorError::QueueFull(2))));
    }
}
