//! Stable **Event Bus API** — ActionLog + orchestration events.
//!
//! This module defines the [`EventBusApi`] and [`OrchestratorBusApi`] traits
//! that any event-bus implementation in infinityOS must satisfy.
//!
//! ## Stability guarantee
//!
//! Both traits are versioned at
//! [`EVENT_BUS_API_VERSION`](super::versioning::EVENT_BUS_API_VERSION).
//! Any breaking change requires a major-version bump and a migration guide
//! in `docs/architecture/deprecation-policy.md`.
//!
//! ## Reference implementation
//!
//! - [`ActionLog`](ify_controller::action_log::ActionLog) in `ify-controller`
//!   implements [`EventBusApi`].
//! - [`LocalOrchestrator`](ify_controller::orchestrator::LocalOrchestrator)
//!   in `ify-controller` implements [`OrchestratorBusApi`].

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// EventBusApi
// ---------------------------------------------------------------------------

/// Stable trait for the cross-layer event bus (ActionLog).
///
/// Implementors provide an append-only, broadcast-capable event sink.
/// All methods are infallible; persistence and delivery-guarantee concerns
/// are handled by adapters downstream of the bus.
///
/// ## Semver contract
///
/// Versioned at [`EVENT_BUS_API_VERSION`](super::versioning::EVENT_BUS_API_VERSION) `1.0.0`.
pub trait EventBusApi: Send + Sync {
    /// The concrete entry type stored and broadcast by this bus.
    type Entry: Clone + Send + Sync + 'static;

    /// Append `entry` to the log and broadcast it to all current subscribers.
    fn append(&self, entry: Self::Entry);

    /// Subscribe to new entries.
    ///
    /// The returned receiver only sees entries appended *after* this call.
    /// Use [`entries_for_dimension`](Self::entries_for_dimension) or
    /// [`all_entries`](Self::all_entries) to replay history.
    fn subscribe(&self) -> broadcast::Receiver<Self::Entry>;

    /// Return all entries scoped to `dim`.
    fn entries_for_dimension(&self, dim: DimensionId) -> Vec<Self::Entry>;

    /// Return all entries associated with `task_id`.
    fn entries_for_task(&self, task_id: TaskId) -> Vec<Self::Entry>;

    /// Return a snapshot of **all** stored entries (oldest first).
    fn all_entries(&self) -> Vec<Self::Entry>;

    /// Number of entries currently stored.
    fn len(&self) -> usize;

    /// Returns `true` when no entries have been recorded.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// OrchestratorBusApi
// ---------------------------------------------------------------------------

/// Kind of event emitted by the orchestrator bus.
///
/// Mirrors the concrete variants in `ify-controller`'s `OrchestratorEvent`
/// at a summary level usable by all layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestratorEventKind {
    /// Task accepted into the orchestrator.
    Submitted,
    /// Incremental progress update.
    Progress,
    /// Task completed successfully.
    Completed,
    /// Task terminated with an error.
    Failed,
    /// Task cancelled by caller or policy.
    Cancelled,
}

/// Stable trait for the cross-layer orchestration event bus.
///
/// Implementors coordinate task lifecycle events (submit, progress, complete,
/// fail, cancel, replay) across layers.
///
/// ## Semver contract
///
/// Versioned at [`EVENT_BUS_API_VERSION`](super::versioning::EVENT_BUS_API_VERSION) `1.0.0`.
pub trait OrchestratorBusApi: Send + Sync {
    /// The concrete event type broadcast by this orchestrator.
    type Event: Clone + Send + Sync + 'static;

    /// The error type returned by mutating operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Submit a new task with the given `task_id` and `dimension_id`.
    fn submit(
        &self,
        task_id: TaskId,
        dimension_id: DimensionId,
        priority: u8,
        payload: serde_json::Value,
    ) -> Result<(), Self::Error>;

    /// Publish an incremental progress event for `task_id`.
    fn progress(
        &self,
        task_id: TaskId,
        percent: u8,
        message: &str,
    ) -> Result<(), Self::Error>;

    /// Mark `task_id` as successfully completed.
    fn complete(&self, task_id: TaskId) -> Result<(), Self::Error>;

    /// Mark `task_id` as failed with `error`.
    fn fail(&self, task_id: TaskId, error: &str) -> Result<(), Self::Error>;

    /// Cancel `task_id`.
    fn cancel(&self, task_id: TaskId) -> Result<(), Self::Error>;

    /// Replay all stored events for `task_id` in order.
    fn replay(&self, task_id: TaskId) -> Result<Vec<Self::Event>, Self::Error>;

    /// Subscribe to all events broadcast by this orchestrator.
    fn subscribe(&self) -> broadcast::Receiver<Self::Event>;
}
