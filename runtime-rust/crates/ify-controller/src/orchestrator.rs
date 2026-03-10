//! Orchestrator dispatch hooks: submit, subscribe to progress, cancel, replay.
//!
//! Satisfies Epic B requirement:
//! > Implement orchestrator dispatch hooks (submit tasks, subscribe to
//! > progress, cancel, replay).
//!
//! ## Design
//!
//! [`LocalOrchestrator`] is a dimension-scoped coordinator that:
//!
//! 1. Accepts task submissions and emits [`OrchestratorEvent::Submitted`].
//! 2. Broadcasts per-task progress events to all subscribers.
//! 3. Accepts cancellation requests, transitioning a task to
//!    [`OrchestratorEvent::Cancelled`].
//! 4. Replays the stored event history for any previously submitted task.
//!
//! The orchestrator deliberately decouples **dispatch** (routing and events)
//! from **execution** (the `ify-executor` crate manages actual async futures).
//! Production deployments wire the two together by having the executor post
//! progress events back into the orchestrator via [`LocalOrchestrator::progress`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use tracing::{debug, info, instrument, warn};

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the orchestrator.
#[derive(Debug, Error)]
pub enum OrchestratorError {
    /// The referenced task does not exist.
    #[error("task {0} not found in orchestrator")]
    TaskNotFound(TaskId),

    /// The task has already reached a terminal state.
    #[error("task {0} is already in a terminal state")]
    AlreadyTerminal(TaskId),

    /// The dimension does not match this orchestrator's dimension.
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch {
        /// Orchestrator's dimension.
        expected: DimensionId,
        /// Caller-supplied dimension.
        got: DimensionId,
    },
}

// ---------------------------------------------------------------------------
// OrchestratorEvent
// ---------------------------------------------------------------------------

/// Events emitted by the orchestrator for a specific task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrchestratorEvent {
    /// Task was accepted into the orchestrator.
    Submitted {
        /// Task identifier.
        task_id: TaskId,
    },
    /// Incremental progress update.
    Progress {
        /// Task identifier.
        task_id: TaskId,
        /// Completion percentage (0–100).
        percent: u8,
        /// Human-readable status message.
        message: String,
    },
    /// Task completed successfully.
    Completed {
        /// Task identifier.
        task_id: TaskId,
    },
    /// Task terminated with an error.
    Failed {
        /// Task identifier.
        task_id: TaskId,
        /// Error description.
        error: String,
    },
    /// Task was cancelled before completion.
    Cancelled {
        /// Task identifier.
        task_id: TaskId,
    },
}

impl OrchestratorEvent {
    /// Return the task ID carried by this event.
    pub fn task_id(&self) -> TaskId {
        match self {
            Self::Submitted { task_id }
            | Self::Progress { task_id, .. }
            | Self::Completed { task_id }
            | Self::Failed { task_id, .. }
            | Self::Cancelled { task_id } => *task_id,
        }
    }

    /// Return `true` if this event represents a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }
}

// ---------------------------------------------------------------------------
// Task record
// ---------------------------------------------------------------------------

struct TaskRecord {
    /// All events emitted for this task (ordered).
    history: Vec<OrchestratorEvent>,
    /// Whether the task has reached a terminal state.
    terminal: bool,
}

// ---------------------------------------------------------------------------
// LocalOrchestrator
// ---------------------------------------------------------------------------

/// Dimension-scoped orchestrator providing dispatch hooks.
///
/// ## Thread safety
///
/// All methods take `&self` and use internal locking; the orchestrator is safe
/// to share across threads via [`Arc<LocalOrchestrator>`].
pub struct LocalOrchestrator {
    dimension_id: DimensionId,
    tasks: Mutex<HashMap<TaskId, TaskRecord>>,
    tx: broadcast::Sender<OrchestratorEvent>,
    action_log: Arc<ActionLog>,
}

impl std::fmt::Debug for LocalOrchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.tasks.lock().map(|g| g.len()).unwrap_or(0);
        write!(
            f,
            "LocalOrchestrator {{ dimension: {}, tasks: {count} }}",
            self.dimension_id
        )
    }
}

impl LocalOrchestrator {
    /// Create a new orchestrator for the given dimension.
    ///
    /// `channel_capacity` sets the broadcast ring-buffer size.
    pub fn new(
        dimension_id: DimensionId,
        action_log: Arc<ActionLog>,
        channel_capacity: usize,
    ) -> Arc<Self> {
        let (tx, _) = broadcast::channel(channel_capacity.max(1));
        Arc::new(Self {
            dimension_id,
            tasks: Mutex::new(HashMap::new()),
            tx,
            action_log,
        })
    }

    // ------------------------------------------------------------------
    // Dispatch hooks
    // ------------------------------------------------------------------

    /// Submit a task to the orchestrator.
    ///
    /// Emits [`OrchestratorEvent::Submitted`] and an ActionLog entry.
    ///
    /// # Errors
    ///
    /// Returns [`OrchestratorError::DimensionMismatch`] if `dimension_id`
    /// does not match this orchestrator's dimension.
    #[instrument(skip(self), fields(dimension = %self.dimension_id, task_id = %task_id))]
    pub fn submit(
        &self,
        task_id: TaskId,
        dimension_id: DimensionId,
    ) -> Result<(), OrchestratorError> {
        if dimension_id != self.dimension_id {
            return Err(OrchestratorError::DimensionMismatch {
                expected: self.dimension_id,
                got: dimension_id,
            });
        }

        let event = OrchestratorEvent::Submitted { task_id };

        {
            let mut tasks = self.tasks.lock().expect("orchestrator lock poisoned");
            tasks.insert(
                task_id,
                TaskRecord {
                    history: vec![event.clone()],
                    terminal: false,
                },
            );
        }

        self.broadcast(event);

        info!(task_id = %task_id, "task submitted to orchestrator");
        self.action_log.append(ActionLogEntry::new(
            EventType::OrchestratorSubmit,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string() }),
        ));

        Ok(())
    }

    /// Publish a progress update for an active task.
    ///
    /// # Errors
    ///
    /// - [`OrchestratorError::TaskNotFound`] if the task is unknown.
    /// - [`OrchestratorError::AlreadyTerminal`] if the task has already ended.
    #[instrument(skip(self, message), fields(task_id = %task_id, percent))]
    pub fn progress(
        &self,
        task_id: TaskId,
        percent: u8,
        message: impl Into<String>,
    ) -> Result<(), OrchestratorError> {
        let message = message.into();
        let event = OrchestratorEvent::Progress {
            task_id,
            percent,
            message: message.clone(),
        };
        self.record_event(task_id, event.clone())?;
        self.broadcast(event);

        debug!(task_id = %task_id, percent, %message, "orchestrator progress");
        self.action_log.append(ActionLogEntry::new(
            EventType::OrchestratorProgress,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string(), "percent": percent, "message": message }),
        ));

        Ok(())
    }

    /// Mark a task as completed.
    ///
    /// # Errors
    ///
    /// - [`OrchestratorError::TaskNotFound`] / [`OrchestratorError::AlreadyTerminal`].
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub fn complete(&self, task_id: TaskId) -> Result<(), OrchestratorError> {
        let event = OrchestratorEvent::Completed { task_id };
        self.record_terminal(task_id, event.clone())?;
        self.broadcast(event);

        info!(task_id = %task_id, "orchestrator task completed");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskCompleted,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string() }),
        ));

        Ok(())
    }

    /// Mark a task as failed.
    ///
    /// # Errors
    ///
    /// - [`OrchestratorError::TaskNotFound`] / [`OrchestratorError::AlreadyTerminal`].
    #[instrument(skip(self, error), fields(task_id = %task_id))]
    pub fn fail(&self, task_id: TaskId, error: impl Into<String>) -> Result<(), OrchestratorError> {
        let error = error.into();
        let event = OrchestratorEvent::Failed {
            task_id,
            error: error.clone(),
        };
        self.record_terminal(task_id, event.clone())?;
        self.broadcast(event);

        warn!(task_id = %task_id, %error, "orchestrator task failed");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskFailed,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string(), "error": error }),
        ));

        Ok(())
    }

    /// Cancel a task.
    ///
    /// # Errors
    ///
    /// - [`OrchestratorError::TaskNotFound`] if the task is unknown.
    /// - [`OrchestratorError::AlreadyTerminal`] if already ended.
    #[instrument(skip(self), fields(task_id = %task_id))]
    pub fn cancel(&self, task_id: TaskId) -> Result<(), OrchestratorError> {
        let event = OrchestratorEvent::Cancelled { task_id };
        self.record_terminal(task_id, event.clone())?;
        self.broadcast(event);

        info!(task_id = %task_id, "orchestrator task cancelled");
        self.action_log.append(ActionLogEntry::new(
            EventType::OrchestratorCancel,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string() }),
        ));

        Ok(())
    }

    /// Replay the stored event history for `task_id`.
    ///
    /// Returns events in submission order.
    ///
    /// # Errors
    ///
    /// - [`OrchestratorError::TaskNotFound`] if the task is not known.
    pub fn replay(&self, task_id: TaskId) -> Result<Vec<OrchestratorEvent>, OrchestratorError> {
        let tasks = self.tasks.lock().expect("orchestrator lock poisoned");
        let record = tasks
            .get(&task_id)
            .ok_or(OrchestratorError::TaskNotFound(task_id))?;
        let events = record.history.clone();
        drop(tasks);

        self.action_log.append(ActionLogEntry::new(
            EventType::OrchestratorReplay,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({
                "task_id": task_id.to_string(),
                "event_count": events.len(),
            }),
        ));

        Ok(events)
    }

    /// Subscribe to all orchestrator events for this dimension.
    ///
    /// The receiver sees events emitted *after* this call.  Use
    /// [`replay`][LocalOrchestrator::replay] to access historical events.
    pub fn subscribe(&self) -> broadcast::Receiver<OrchestratorEvent> {
        self.tx.subscribe()
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn broadcast(&self, event: OrchestratorEvent) {
        if let Err(e) = self.tx.send(event) {
            if self.tx.receiver_count() > 0 {
                warn!("orchestrator broadcast failed: {e}");
            }
        }
    }

    fn record_event(
        &self,
        task_id: TaskId,
        event: OrchestratorEvent,
    ) -> Result<(), OrchestratorError> {
        let mut tasks = self.tasks.lock().expect("orchestrator lock poisoned");
        let record = tasks
            .get_mut(&task_id)
            .ok_or(OrchestratorError::TaskNotFound(task_id))?;
        if record.terminal {
            return Err(OrchestratorError::AlreadyTerminal(task_id));
        }
        record.history.push(event);
        Ok(())
    }

    fn record_terminal(
        &self,
        task_id: TaskId,
        event: OrchestratorEvent,
    ) -> Result<(), OrchestratorError> {
        let mut tasks = self.tasks.lock().expect("orchestrator lock poisoned");
        let record = tasks
            .get_mut(&task_id)
            .ok_or(OrchestratorError::TaskNotFound(task_id))?;
        if record.terminal {
            return Err(OrchestratorError::AlreadyTerminal(task_id));
        }
        record.history.push(event);
        record.terminal = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_orchestrator() -> Arc<LocalOrchestrator> {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        LocalOrchestrator::new(dim, log, 16)
    }

    #[test]
    fn submit_and_complete() {
        let orch = make_orchestrator();
        let task = TaskId::new();

        orch.submit(task, orch.dimension_id).unwrap();
        orch.progress(task, 50, "halfway").unwrap();
        orch.complete(task).unwrap();

        let history = orch.replay(task).unwrap();
        assert_eq!(history.len(), 3);
        assert!(history[2].is_terminal());
    }

    #[test]
    fn cancel_prevents_further_updates() {
        let orch = make_orchestrator();
        let task = TaskId::new();

        orch.submit(task, orch.dimension_id).unwrap();
        orch.cancel(task).unwrap();

        let err = orch.progress(task, 10, "nope");
        assert!(matches!(err, Err(OrchestratorError::AlreadyTerminal(_))));
    }

    #[test]
    fn submit_dimension_mismatch_fails() {
        let orch = make_orchestrator();
        let task = TaskId::new();
        let wrong_dim = DimensionId::new();

        let err = orch.submit(task, wrong_dim);
        assert!(matches!(err, Err(OrchestratorError::DimensionMismatch { .. })));
    }

    #[test]
    fn replay_unknown_task_fails() {
        let orch = make_orchestrator();
        let task = TaskId::new();
        let err = orch.replay(task);
        assert!(matches!(err, Err(OrchestratorError::TaskNotFound(_))));
    }

    #[test]
    fn fail_task_emits_failed_event() {
        let orch = make_orchestrator();
        let task = TaskId::new();

        orch.submit(task, orch.dimension_id).unwrap();
        orch.fail(task, "something broke").unwrap();

        let history = orch.replay(task).unwrap();
        assert!(matches!(history.last(), Some(OrchestratorEvent::Failed { .. })));
    }

    #[tokio::test]
    async fn subscribe_receives_events() {
        let orch = make_orchestrator();
        let mut rx = orch.subscribe();
        let task = TaskId::new();

        orch.submit(task, orch.dimension_id).unwrap();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, OrchestratorEvent::Submitted { .. }));
    }
}
