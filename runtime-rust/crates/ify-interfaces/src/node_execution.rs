//! Stable **Node Execution API** — planner → executor → reporter.
//!
//! This module defines three complementary traits that together cover the
//! full node execution lifecycle:
//!
//! 1. [`NodePlannerApi`] — converts a graph into an ordered execution plan.
//! 2. [`NodeExecutorApi`] — drives execution of a planned task.
//! 3. [`NodeReporterApi`] — reports progress and terminal status for a running task.
//!
//! ## Pipeline overview
//!
//! ```text
//!   ┌──────────────┐     plan()     ┌────────────────┐
//!   │ NodePlannerApi│ ────────────► │ NodeExecutorApi │
//!   └──────────────┘                └────────┬───────┘
//!                                            │ progress() / complete() / fail()
//!                                            ▼
//!                                   ┌─────────────────┐
//!                                   │ NodeReporterApi  │
//!                                   └─────────────────┘
//! ```
//!
//! ## Stability guarantee
//!
//! All three traits are versioned at
//! [`NODE_EXECUTION_API_VERSION`](super::versioning::NODE_EXECUTION_API_VERSION).
//!
//! ## Reference implementation
//!
//! - [`FlowGraph`](ify_controller::graph::FlowGraph) provides the planner
//!   (topological ordering) in `ify-controller`.
//! - [`LocalOrchestrator`](ify_controller::orchestrator::LocalOrchestrator)
//!   implements both [`NodeExecutorApi`] and [`NodeReporterApi`].

use ify_core::{DimensionId, TaskId};

// ---------------------------------------------------------------------------
// NodePlannerApi
// ---------------------------------------------------------------------------

/// Stable trait for the node execution planner.
///
/// The planner accepts a graph representation and returns an ordered execution
/// plan that the executor can drive step-by-step.
///
/// ## Semver contract
///
/// Versioned at
/// [`NODE_EXECUTION_API_VERSION`](super::versioning::NODE_EXECUTION_API_VERSION) `1.0.0`.
pub trait NodePlannerApi: Send + Sync {
    /// The execution plan produced by the planner (e.g., a topological order).
    type Plan: Clone + Send + Sync + 'static;

    /// The error type returned when planning fails.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Produce an execution plan for the graph owned by `dimension_id`.
    ///
    /// Returns [`Err`] if the graph contains a cycle, has unresolved
    /// dependencies, or fails validation.
    fn plan(&self, dimension_id: DimensionId) -> Result<Self::Plan, Self::Error>;

    /// Validate the graph without producing an executable plan.
    ///
    /// Returns `Ok(())` if the graph is valid, `Err` with a description
    /// of every issue found.
    fn validate(&self, dimension_id: DimensionId) -> Result<(), Vec<String>>;
}

// ---------------------------------------------------------------------------
// NodeExecutorApi
// ---------------------------------------------------------------------------

/// Stable trait for the node task executor.
///
/// The executor drives a planned task to completion, delegating work to
/// node-specific runners and propagating status via [`NodeReporterApi`].
///
/// ## Semver contract
///
/// Versioned at
/// [`NODE_EXECUTION_API_VERSION`](super::versioning::NODE_EXECUTION_API_VERSION) `1.0.0`.
pub trait NodeExecutorApi: Send + Sync {
    /// The error type returned by executor operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Submit `task_id` to the executor for the given `dimension_id`.
    ///
    /// The executor enqueues the task and begins execution according to the
    /// scheduling policy.  Progress is reported via the associated
    /// [`NodeReporterApi`] implementation.
    fn submit(
        &self,
        task_id: TaskId,
        dimension_id: DimensionId,
        priority: u8,
        payload: serde_json::Value,
    ) -> Result<(), Self::Error>;

    /// Request cancellation of `task_id`.
    ///
    /// This is a best-effort signal; in-flight work may not stop immediately.
    fn cancel(&self, task_id: TaskId) -> Result<(), Self::Error>;
}

// ---------------------------------------------------------------------------
// NodeReporterApi
// ---------------------------------------------------------------------------

/// Stable trait for reporting node execution status.
///
/// The reporter decouples the execution engine from status consumers
/// (canvas overlays, ActionLog, telemetry).  All three terminal transitions
/// (`complete`, `fail`, `cancel`) are idempotent for the reporter: a second
/// call for the same `task_id` after a terminal event should be a no-op or
/// return an appropriate error.
///
/// ## Semver contract
///
/// Versioned at
/// [`NODE_EXECUTION_API_VERSION`](super::versioning::NODE_EXECUTION_API_VERSION) `1.0.0`.
pub trait NodeReporterApi: Send + Sync {
    /// The error type returned by reporting operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Emit an incremental progress update for `task_id`.
    ///
    /// `percent` must be in the range `0..=100`.
    fn progress(
        &self,
        task_id: TaskId,
        percent: u8,
        message: &str,
    ) -> Result<(), Self::Error>;

    /// Mark `task_id` as successfully completed.
    fn complete(&self, task_id: TaskId) -> Result<(), Self::Error>;

    /// Mark `task_id` as failed with `error_message`.
    fn fail(
        &self,
        task_id: TaskId,
        error_message: &str,
    ) -> Result<(), Self::Error>;

    /// Mark `task_id` as cancelled.
    fn cancel(&self, task_id: TaskId) -> Result<(), Self::Error>;
}
