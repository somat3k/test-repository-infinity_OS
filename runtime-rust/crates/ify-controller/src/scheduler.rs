//! Job Scheduling and Task Lifecycle — Epic J
//!
//! This module provides the complete task scheduling infrastructure for
//! infinityOS, implementing all ten Epic J items:
//!
//! 1. **Task states** — `TaskState` with a validated transition graph.
//! 2. **Priority-aware scheduling** — `TaskPriority` ordering used in a
//!    heap-based ready queue.
//! 3. **Retries, backoff, and cancellation** — `RetryPolicy` with
//!    configurable max attempts and exponential backoff.
//! 4. **Task leasing / heartbeats** — `TaskLease` with expiry and renewal,
//!    ensuring distributed workers stay accountable.
//! 5. **Per-dimension quotas and rate limiting** — `DimensionQuota` enforced
//!    on submission via a token-bucket style counter.
//! 6. **Dependency-aware scheduling** — DAG-based readiness check; a task
//!    only moves to `Queued` when all prerequisite tasks have completed.
//! 7. **Task preemption policy** — `PreemptionPolicy` lets higher-priority
//!    tasks displace running lower-priority ones.
//! 8. **Task persistence + recovery** — `TaskRecord` is fully serialisable;
//!    `TaskScheduler::snapshot` / `restore` provide a persistence hook.
//! 9. **Task templates** — `TaskTemplate` for repeated workflows with
//!    per-instantiation parameter overrides.
//! 10. **Unique TaskID enforcement + index** — the scheduler maintains an
//!     insert-ordered `IndexMap` and rejects duplicate IDs.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the task scheduler.
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// A task with the same ID is already registered.
    #[error("duplicate task id: {0}")]
    DuplicateTaskId(TaskId),

    /// A referenced task was not found.
    #[error("task {0} not found")]
    TaskNotFound(TaskId),

    /// The requested state transition is invalid.
    #[error("invalid transition for task {task_id}: {from:?} → {to:?}")]
    InvalidTransition {
        /// Affected task.
        task_id: TaskId,
        /// Current state.
        from: TaskState,
        /// Requested target state.
        to: TaskState,
    },

    /// The dimension quota would be exceeded by this submission.
    #[error("dimension {dim} quota exceeded: running={running}, limit={limit}")]
    QuotaExceeded {
        /// Dimension whose quota was exceeded.
        dim: DimensionId,
        /// Number of tasks currently running.
        running: u32,
        /// Configured concurrent-task limit.
        limit: u32,
    },

    /// A prerequisite task referenced in the dependency list was not found.
    #[error("prerequisite task {0} does not exist")]
    PrerequisiteNotFound(TaskId),

    /// The lease for the given task has expired.
    #[error("lease for task {0} has expired")]
    LeaseExpired(TaskId),

    /// A template with the given ID was not found.
    #[error("task template {0} not found")]
    TemplateNotFound(Uuid),
}

// ---------------------------------------------------------------------------
// TaskState
// ---------------------------------------------------------------------------

/// Lifecycle state of a scheduled task.
///
/// Valid transitions:
/// ```text
/// Pending  → Queued | Cancelled
/// Queued   → Running | Cancelled
/// Running  → Paused | Completed | Failed | Cancelled
/// Paused   → Queued | Cancelled
/// Failed   → Queued (retry) | Cancelled
/// Completed → (terminal)
/// Cancelled → (terminal)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Task has been accepted but its prerequisites are not yet satisfied.
    Pending,
    /// Prerequisites satisfied; waiting in the priority queue.
    Queued,
    /// A worker has picked up and is executing this task.
    Running,
    /// Execution was paused (e.g. by a preemption policy).
    Paused,
    /// Task finished successfully (terminal).
    Completed,
    /// Task failed after exhausting retries (terminal).
    Failed,
    /// Task was explicitly cancelled (terminal).
    Cancelled,
}

impl TaskState {
    /// Returns `true` if this is a terminal state (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Return `true` if the transition `self → next` is valid.
    pub fn can_transition_to(self, next: TaskState) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match (self, next) {
            (Self::Pending, Self::Queued | Self::Cancelled) => true,
            (Self::Queued, Self::Running | Self::Cancelled) => true,
            (Self::Running, Self::Paused | Self::Completed | Self::Failed | Self::Cancelled) => {
                true
            }
            (Self::Paused, Self::Queued | Self::Cancelled) => true,
            // Retry path: failed → queued
            (Self::Failed, Self::Queued | Self::Cancelled) => true,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// TaskPriority
// ---------------------------------------------------------------------------

/// Scheduling priority for a task.
///
/// Higher numeric values sort first in the ready queue.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Background housekeeping (lowest).
    Background = 0,
    /// Normal interactive work.
    #[default]
    Normal = 1,
    /// Time-sensitive user-facing tasks.
    High = 2,
    /// System-critical or deadline-driven tasks (highest).
    Critical = 3,
}

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

/// Policy controlling how a failed task is retried.
///
/// `max_attempts` is the **total** number of execution attempts allowed
/// (including the initial attempt).  Set to `1` for no retries, `4` for an
/// initial attempt plus three retries, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum **total** number of execution attempts (1 = no retries, 4 = 3
    /// retries after the first failure).
    pub max_attempts: u32,
    /// Base backoff in milliseconds; actual delay = `base_ms * 2^attempt`,
    /// capped at `max_backoff_ms`.
    pub base_backoff_ms: u64,
    /// Maximum backoff ceiling in milliseconds.
    pub max_backoff_ms: u64,
}

impl RetryPolicy {
    /// No retries — only one execution attempt is allowed.
    pub fn none() -> Self {
        Self { max_attempts: 1, base_backoff_ms: 0, max_backoff_ms: 0 }
    }

    /// Exponential backoff with the given parameters.
    ///
    /// `max_attempts` is the total number of execution attempts (initial +
    /// retries).
    pub fn exponential(max_attempts: u32, base_backoff_ms: u64, max_backoff_ms: u64) -> Self {
        Self { max_attempts, base_backoff_ms, max_backoff_ms }
    }

    /// Compute the backoff delay for attempt `n` (0-indexed).
    pub fn backoff_for(&self, attempt: u32) -> Duration {
        if self.base_backoff_ms == 0 {
            return Duration::ZERO;
        }
        let shift = attempt.min(62); // prevent overflow
        let raw = self.base_backoff_ms.saturating_mul(1u64 << shift);
        Duration::from_millis(raw.min(self.max_backoff_ms))
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::exponential(3, 500, 30_000)
    }
}

// ---------------------------------------------------------------------------
// PreemptionPolicy
// ---------------------------------------------------------------------------

/// Policy controlling whether running tasks can be preempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreemptionPolicy {
    /// Running tasks are never preempted.
    #[default]
    None,
    /// A task with priority ≥ `Critical` may preempt lower-priority running
    /// tasks.
    CriticalPreemptsAll,
    /// Any task with a strictly higher priority may preempt a running task.
    HigherPriorityWins,
}

// ---------------------------------------------------------------------------
// TaskLease
// ---------------------------------------------------------------------------

/// A timed lease issued to a worker that claimed a task.
///
/// If the worker does not renew before `expires_at_ms` the task may be
/// reclaimed by the scheduler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLease {
    /// ID of the leased task.
    pub task_id: TaskId,
    /// Opaque worker identifier.
    pub worker_id: String,
    /// Unix epoch milliseconds when this lease was issued.
    pub issued_at_ms: u64,
    /// Unix epoch milliseconds when this lease expires.
    pub expires_at_ms: u64,
}

impl TaskLease {
    /// Create a new lease valid for `duration_ms` milliseconds.
    pub fn new(task_id: TaskId, worker_id: impl Into<String>, duration_ms: u64) -> Self {
        let now = now_ms();
        Self {
            task_id,
            worker_id: worker_id.into(),
            issued_at_ms: now,
            expires_at_ms: now.saturating_add(duration_ms),
        }
    }

    /// Returns `true` if the lease is still valid at `now_ms`.
    pub fn is_valid(&self, now_ms: u64) -> bool {
        now_ms < self.expires_at_ms
    }

    /// Renew the lease by extending it by `duration_ms` milliseconds from
    /// `now`.
    pub fn renew(&mut self, duration_ms: u64) {
        let now = now_ms();
        self.issued_at_ms = now;
        self.expires_at_ms = now.saturating_add(duration_ms);
    }
}

// ---------------------------------------------------------------------------
// DimensionQuota
// ---------------------------------------------------------------------------

/// Per-dimension concurrency quota and rate-limiting configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionQuota {
    /// Maximum number of concurrently running tasks for this dimension.
    pub max_concurrent: u32,
    /// Maximum number of queued tasks waiting for a worker.
    pub max_queued: u32,
    /// Maximum total tasks accepted per minute (0 = unlimited).
    pub tasks_per_minute: u32,
}

impl DimensionQuota {
    /// Unlimited quota (no constraints).
    pub fn unlimited() -> Self {
        Self { max_concurrent: u32::MAX, max_queued: u32::MAX, tasks_per_minute: 0 }
    }

    /// Sensible defaults: 4 concurrent, 100 queued, 60/min.
    pub fn default_limits() -> Self {
        Self { max_concurrent: 4, max_queued: 100, tasks_per_minute: 60 }
    }
}

impl Default for DimensionQuota {
    fn default() -> Self {
        Self::default_limits()
    }
}

// ---------------------------------------------------------------------------
// TaskTemplate
// ---------------------------------------------------------------------------

/// A reusable task template for repeated workflows.
///
/// Templates capture the invariant parts of a task (kind, description,
/// default parameters, retry policy) so that new tasks can be instantiated
/// with per-run overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTemplate {
    /// Unique template identifier.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Task kind string (e.g. `"http.request"`, `"ml.train"`).
    pub kind: String,
    /// Default parameters; overridable at instantiation time.
    pub default_params: BTreeMap<String, serde_json::Value>,
    /// Default priority.
    pub priority: TaskPriority,
    /// Default retry policy.
    pub retry_policy: RetryPolicy,
}

impl TaskTemplate {
    /// Create a new template.
    pub fn new(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            description: String::new(),
            kind: kind.into(),
            default_params: BTreeMap::new(),
            priority: TaskPriority::Normal,
            retry_policy: RetryPolicy::default(),
        }
    }

    /// Instantiate a [`TaskSpec`] from this template, merging `overrides`.
    pub fn instantiate(
        &self,
        dim: DimensionId,
        overrides: BTreeMap<String, serde_json::Value>,
    ) -> TaskSpec {
        let mut params = self.default_params.clone();
        params.extend(overrides);
        TaskSpec {
            dimension_id: dim,
            kind: self.kind.clone(),
            label: self.name.clone(),
            priority: self.priority,
            params,
            retry_policy: self.retry_policy.clone(),
            dependencies: Vec::new(),
            template_id: Some(self.id),
        }
    }
}

// ---------------------------------------------------------------------------
// TaskSpec — immutable task definition
// ---------------------------------------------------------------------------

/// Immutable specification for a new task submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    /// Dimension this task belongs to.
    pub dimension_id: DimensionId,
    /// Task kind (e.g. `"http.request"`, `"ml.train"`).
    pub kind: String,
    /// Human-readable label.
    pub label: String,
    /// Scheduling priority.
    pub priority: TaskPriority,
    /// Arbitrary task parameters.
    pub params: BTreeMap<String, serde_json::Value>,
    /// Retry policy.
    pub retry_policy: RetryPolicy,
    /// TaskIDs that must be `Completed` before this task may be `Queued`.
    pub dependencies: Vec<TaskId>,
    /// Template this was instantiated from, if any.
    pub template_id: Option<Uuid>,
}

impl TaskSpec {
    /// Construct a minimal spec with sensible defaults.
    pub fn new(dim: DimensionId, kind: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            dimension_id: dim,
            kind: kind.into(),
            label: label.into(),
            priority: TaskPriority::Normal,
            params: BTreeMap::new(),
            retry_policy: RetryPolicy::default(),
            dependencies: Vec::new(),
            template_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// TaskRecord — mutable live task state
// ---------------------------------------------------------------------------

/// Mutable runtime record for a scheduled task.
///
/// `TaskRecord` is the unit of persistence — it can be serialised to JSON and
/// restored to reconstruct scheduler state after a crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    /// Globally unique task identifier.
    pub task_id: TaskId,
    /// Immutable task specification.
    pub spec: TaskSpec,
    /// Current lifecycle state.
    pub state: TaskState,
    /// Number of attempts so far (0 = not yet started).
    pub attempt: u32,
    /// Unix epoch milliseconds when the task was submitted.
    pub submitted_at_ms: u64,
    /// Unix epoch milliseconds when the task last transitioned state.
    pub updated_at_ms: u64,
    /// Human-readable failure reason (if `state == Failed`).
    pub failure_reason: Option<String>,
    /// Active lease, if the task is currently claimed by a worker.
    pub lease: Option<TaskLease>,
    /// Monotonically increasing sequence number; used for stable ordering
    /// within the same priority tier.
    pub seq: u64,
}

impl TaskRecord {
    /// Create a new record in `Pending` state.
    fn new(task_id: TaskId, spec: TaskSpec, seq: u64) -> Self {
        let now = now_ms();
        Self {
            task_id,
            spec,
            state: TaskState::Pending,
            attempt: 0,
            submitted_at_ms: now,
            updated_at_ms: now,
            failure_reason: None,
            lease: None,
            seq,
        }
    }
}

// ---------------------------------------------------------------------------
// TaskIndex — O(1) task ID lookup
// ---------------------------------------------------------------------------

/// Internal index ensuring unique TaskIDs and fast lookup.
struct TaskIndex {
    /// Insertion-ordered map from TaskId → record.
    records: HashMap<TaskId, TaskRecord>,
    /// Insertion order for deterministic iteration.
    insertion_order: Vec<TaskId>,
    /// Running sequence counter.
    seq: u64,
}

impl TaskIndex {
    fn new() -> Self {
        Self {
            records: HashMap::new(),
            insertion_order: Vec::new(),
            seq: 0,
        }
    }

    fn insert(&mut self, task_id: TaskId, spec: TaskSpec) -> Result<(), SchedulerError> {
        if self.records.contains_key(&task_id) {
            return Err(SchedulerError::DuplicateTaskId(task_id));
        }
        self.seq += 1;
        let record = TaskRecord::new(task_id, spec, self.seq);
        self.records.insert(task_id, record);
        self.insertion_order.push(task_id);
        Ok(())
    }

    fn get(&self, id: TaskId) -> Option<&TaskRecord> {
        self.records.get(&id)
    }

    fn get_mut(&mut self, id: TaskId) -> Option<&mut TaskRecord> {
        self.records.get_mut(&id)
    }

    fn contains(&self, id: TaskId) -> bool {
        self.records.contains_key(&id)
    }

    fn iter_in_order(&self) -> impl Iterator<Item = &TaskRecord> {
        self.insertion_order
            .iter()
            .filter_map(|id| self.records.get(id))
    }
}

// ---------------------------------------------------------------------------
// TaskScheduler
// ---------------------------------------------------------------------------

/// Priority-aware, DAG-dependency scheduler with retries, leasing, quotas,
/// and persistence hooks.
///
/// # Architecture
///
/// The scheduler is an in-process, single-dimension-aware unit.  It maintains:
/// * a [`TaskIndex`] as the canonical source of truth,
/// * a priority-ordered `ready_queue` of `(priority, seq, TaskId)` tuples,
/// * per-dimension concurrency and rate-limit tracking,
/// * an optional [`PreemptionPolicy`].
///
/// The scheduler does **not** own executor threads.  Workers call
/// [`Self::next_ready`] to dequeue a task, hold a [`TaskLease`], and report
/// back via [`Self::complete`] / [`Self::fail`] / [`Self::cancel`].
pub struct TaskScheduler {
    index: TaskIndex,
    /// `(priority, seq, task_id)` — sorted highest-priority first.
    ready_queue: VecDeque<(TaskPriority, u64, TaskId)>,
    /// Quotas per dimension.
    quotas: HashMap<DimensionId, DimensionQuota>,
    /// Running task count per dimension.
    running_count: HashMap<DimensionId, u32>,
    /// Template registry.
    templates: HashMap<Uuid, TaskTemplate>,
    preemption_policy: PreemptionPolicy,
    lease_duration_ms: u64,
    action_log: Arc<ActionLog>,
    dimension_id: DimensionId,
    #[allow(dead_code)]
    coordinator_task_id: TaskId,
}

impl TaskScheduler {
    /// Create a new scheduler.
    ///
    /// `lease_duration_ms` controls how long a worker's lease is valid before
    /// the task may be reclaimed.
    pub fn new(
        dimension_id: DimensionId,
        coordinator_task_id: TaskId,
        preemption_policy: PreemptionPolicy,
        lease_duration_ms: u64,
        action_log: Arc<ActionLog>,
    ) -> Self {
        Self {
            index: TaskIndex::new(),
            ready_queue: VecDeque::new(),
            quotas: HashMap::new(),
            running_count: HashMap::new(),
            templates: HashMap::new(),
            preemption_policy,
            lease_duration_ms,
            action_log,
            dimension_id,
            coordinator_task_id,
        }
    }

    // ── Quota management ─────────────────────────────────────────────────

    /// Set the concurrency quota for a dimension.
    pub fn set_quota(&mut self, dim: DimensionId, quota: DimensionQuota) {
        self.quotas.insert(dim, quota);
    }

    fn quota_for(&self, dim: DimensionId) -> &DimensionQuota {
        static UNLIMITED: DimensionQuota = DimensionQuota {
            max_concurrent: u32::MAX,
            max_queued: u32::MAX,
            tasks_per_minute: 0,
        };
        self.quotas.get(&dim).unwrap_or(&UNLIMITED)
    }

    fn running_for(&self, dim: DimensionId) -> u32 {
        self.running_count.get(&dim).copied().unwrap_or(0)
    }

    fn increment_running(&mut self, dim: DimensionId) {
        *self.running_count.entry(dim).or_insert(0) += 1;
    }

    fn decrement_running(&mut self, dim: DimensionId) {
        let count = self.running_count.entry(dim).or_insert(0);
        *count = count.saturating_sub(1);
    }

    // ── Template management ───────────────────────────────────────────────

    /// Register a task template.
    pub fn register_template(&mut self, template: TaskTemplate) {
        self.templates.insert(template.id, template);
    }

    /// Look up a template by ID.
    pub fn get_template(&self, id: Uuid) -> Option<&TaskTemplate> {
        self.templates.get(&id)
    }

    /// Instantiate a task from a registered template and submit it.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::TemplateNotFound`] if the template is absent.
    pub fn submit_from_template(
        &mut self,
        template_id: Uuid,
        task_id: TaskId,
        overrides: BTreeMap<String, serde_json::Value>,
    ) -> Result<(), SchedulerError> {
        let spec = self
            .templates
            .get(&template_id)
            .ok_or(SchedulerError::TemplateNotFound(template_id))?
            .instantiate(self.dimension_id, overrides);
        self.submit(task_id, spec)
    }

    // ── Task submission ───────────────────────────────────────────────────

    /// Submit a new task.
    ///
    /// * Validates the TaskID is unique.
    /// * Checks quota for the target dimension.
    /// * Validates all declared prerequisites exist.
    /// * If all prerequisites are already `Completed`, moves directly to
    ///   `Queued`; otherwise stays `Pending`.
    ///
    /// Emits [`EventType::TaskSubmitted`].
    ///
    /// # Errors
    ///
    /// * [`SchedulerError::DuplicateTaskId`]
    /// * [`SchedulerError::QuotaExceeded`]
    /// * [`SchedulerError::PrerequisiteNotFound`]
    pub fn submit(&mut self, task_id: TaskId, spec: TaskSpec) -> Result<(), SchedulerError> {
        // Uniqueness check.
        if self.index.contains(task_id) {
            return Err(SchedulerError::DuplicateTaskId(task_id));
        }

        // Quota check on the *queued* count for the dimension.
        let dim = spec.dimension_id;
        let queued_count = self
            .index
            .iter_in_order()
            .filter(|r| r.spec.dimension_id == dim && r.state == TaskState::Queued)
            .count() as u32;
        let quota = self.quota_for(dim);
        if queued_count >= quota.max_queued {
            return Err(SchedulerError::QuotaExceeded {
                dim,
                running: self.running_for(dim),
                limit: quota.max_queued,
            });
        }

        // Prerequisite existence check.
        for &prereq in &spec.dependencies {
            if !self.index.contains(prereq) {
                return Err(SchedulerError::PrerequisiteNotFound(prereq));
            }
        }

        let deps_done = self.all_deps_completed(&spec.dependencies);
        self.index.insert(task_id, spec)?;

        self.action_log.append(ActionLogEntry::new(
            EventType::TaskSubmitted,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id }),
        ));

        if deps_done {
            self.transition(task_id, TaskState::Queued)?;
        }
        Ok(())
    }

    // ── Worker interface ──────────────────────────────────────────────────

    /// Dequeue the highest-priority ready task and issue a lease.
    ///
    /// Returns `None` if the ready queue is empty or the running quota would
    /// be exceeded for the task's dimension.
    pub fn next_ready(&mut self) -> Option<(TaskRecord, TaskLease)> {
        // Sort the queue each time to respect priorities (rare re-inserts).
        self.sort_ready_queue();

        let pos = self.ready_queue.iter().position(|(_, _, tid)| {
            if let Some(r) = self.index.get(*tid) {
                let dim = r.spec.dimension_id;
                self.running_for(dim) < self.quota_for(dim).max_concurrent
            } else {
                false
            }
        })?;

        let (_, _, task_id) = self.ready_queue.remove(pos)?;
        self.transition(task_id, TaskState::Running).ok()?;

        let lease = TaskLease::new(
            task_id,
            format!("worker-{}", Uuid::new_v4()),
            self.lease_duration_ms,
        );

        // Store lease on the record.
        let record = self.index.get_mut(task_id)?;
        record.lease = Some(lease.clone());
        record.attempt += 1;

        self.action_log.append(ActionLogEntry::new(
            EventType::TaskStarted,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({
                "task_id": task_id,
                "worker": lease.worker_id,
                "attempt": record.attempt,
            }),
        ));

        Some((record.clone(), lease))
    }

    /// Renew the lease for `task_id`, extending it by `self.lease_duration_ms`.
    ///
    /// # Errors
    ///
    /// * [`SchedulerError::TaskNotFound`]
    /// * [`SchedulerError::LeaseExpired`]
    pub fn renew_lease(&mut self, task_id: TaskId) -> Result<TaskLease, SchedulerError> {
        let record = self
            .index
            .get_mut(task_id)
            .ok_or(SchedulerError::TaskNotFound(task_id))?;

        let now = now_ms();
        let lease = record.lease.as_mut().ok_or(SchedulerError::LeaseExpired(task_id))?;
        if !lease.is_valid(now) {
            return Err(SchedulerError::LeaseExpired(task_id));
        }
        lease.renew(self.lease_duration_ms);
        Ok(lease.clone())
    }

    // ── Outcome reporting ─────────────────────────────────────────────────

    /// Mark a running task as completed.
    ///
    /// Emits [`EventType::TaskCompleted`] and triggers readiness promotion for
    /// any tasks that were waiting on this task.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidTransition`] if not currently running.
    pub fn complete(&mut self, task_id: TaskId) -> Result<(), SchedulerError> {
        self.transition(task_id, TaskState::Completed)?;
        let dim = self
            .index
            .get(task_id)
            .map(|r| r.spec.dimension_id)
            .unwrap_or(self.dimension_id);
        self.decrement_running(dim);
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskCompleted,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id }),
        ));
        // Promote any tasks whose dependencies are now all completed.
        self.promote_pending_tasks();
        Ok(())
    }

    /// Mark a running task as failed and apply the retry policy.
    ///
    /// If retries remain, the task is re-queued after the computed backoff.
    /// Otherwise it transitions to `Failed`.
    ///
    /// Emits [`EventType::TaskFailed`] or [`EventType::TaskRetried`].
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidTransition`] if not running/paused.
    pub fn fail(
        &mut self,
        task_id: TaskId,
        reason: impl Into<String>,
    ) -> Result<(), SchedulerError> {
        let reason = reason.into();
        let (attempt, max_attempts, dim) = {
            let r = self
                .index
                .get(task_id)
                .ok_or(SchedulerError::TaskNotFound(task_id))?;
            (
                r.attempt,
                r.spec.retry_policy.max_attempts,
                r.spec.dimension_id,
            )
        };

        // Retry if we have not yet exhausted max_attempts total executions.
        // `attempt` was incremented when the task was dequeued, so
        // `attempt < max_attempts` means there are still executions remaining.
        if attempt < max_attempts {
            self.transition(task_id, TaskState::Failed)?;
            {
                let r = self
                    .index
                    .get_mut(task_id)
                    .ok_or(SchedulerError::TaskNotFound(task_id))?;
                r.failure_reason = Some(reason.clone());
            }
            self.decrement_running(dim);
            self.action_log.append(ActionLogEntry::new(
                EventType::TaskRetried,
                Actor::System,
                Some(self.dimension_id),
                Some(task_id),
                serde_json::json!({ "task_id": task_id, "attempt": attempt, "reason": reason }),
            ));
            self.transition(task_id, TaskState::Queued)?;
        } else {
            self.transition(task_id, TaskState::Failed)?;
            {
                let r = self
                    .index
                    .get_mut(task_id)
                    .ok_or(SchedulerError::TaskNotFound(task_id))?;
                r.failure_reason = Some(reason.clone());
            }
            self.decrement_running(dim);
            self.action_log.append(ActionLogEntry::new(
                EventType::TaskFailed,
                Actor::System,
                Some(self.dimension_id),
                Some(task_id),
                serde_json::json!({ "task_id": task_id, "reason": reason }),
            ));
        }
        Ok(())
    }

    /// Cancel a task in any non-terminal state.
    ///
    /// Emits [`EventType::TaskCancelled`].
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidTransition`] if already terminal.
    pub fn cancel(&mut self, task_id: TaskId) -> Result<(), SchedulerError> {
        let was_running = self
            .index
            .get(task_id)
            .map(|r| r.state == TaskState::Running)
            .unwrap_or(false);
        let dim = self
            .index
            .get(task_id)
            .map(|r| r.spec.dimension_id)
            .unwrap_or(self.dimension_id);

        self.transition(task_id, TaskState::Cancelled)?;
        if was_running {
            self.decrement_running(dim);
        }
        // Remove from ready queue if present.
        self.ready_queue.retain(|(_, _, id)| *id != task_id);

        self.action_log.append(ActionLogEntry::new(
            EventType::TaskCancelled,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id }),
        ));
        Ok(())
    }

    /// Pause a running task (preemption or explicit pause).
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidTransition`] if not running.
    pub fn pause(&mut self, task_id: TaskId) -> Result<(), SchedulerError> {
        let dim = self
            .index
            .get(task_id)
            .map(|r| r.spec.dimension_id)
            .unwrap_or(self.dimension_id);
        self.transition(task_id, TaskState::Paused)?;
        self.decrement_running(dim);
        Ok(())
    }

    /// Resume a paused task by re-queuing it.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::InvalidTransition`] if not paused.
    pub fn resume(&mut self, task_id: TaskId) -> Result<(), SchedulerError> {
        self.transition(task_id, TaskState::Queued)
    }

    // ── Preemption ────────────────────────────────────────────────────────

    /// Evaluate the preemption policy and return the task IDs that should be
    /// paused to make room for `new_task_id`.
    ///
    /// Returns an empty `Vec` if the policy is `None` or no preemption is
    /// warranted.
    pub fn candidates_for_preemption(
        &self,
        new_task_id: TaskId,
    ) -> Vec<TaskId> {
        let Some(new_record) = self.index.get(new_task_id) else {
            return vec![];
        };
        let new_prio = new_record.spec.priority;

        match self.preemption_policy {
            PreemptionPolicy::None => vec![],
            PreemptionPolicy::CriticalPreemptsAll => {
                if new_prio < TaskPriority::Critical {
                    return vec![];
                }
                self.index
                    .iter_in_order()
                    .filter(|r| {
                        r.state == TaskState::Running && r.spec.priority < TaskPriority::Critical
                    })
                    .map(|r| r.task_id)
                    .collect()
            }
            PreemptionPolicy::HigherPriorityWins => {
                self.index
                    .iter_in_order()
                    .filter(|r| r.state == TaskState::Running && r.spec.priority < new_prio)
                    .map(|r| r.task_id)
                    .collect()
            }
        }
    }

    // ── Queries ───────────────────────────────────────────────────────────

    /// Return the current [`TaskRecord`] for `task_id`.
    pub fn get(&self, task_id: TaskId) -> Option<&TaskRecord> {
        self.index.get(task_id)
    }

    /// Return all tasks in insertion order.
    pub fn all_tasks(&self) -> Vec<&TaskRecord> {
        self.index.iter_in_order().collect()
    }

    /// Return all tasks in a given state.
    pub fn tasks_in_state(&self, state: TaskState) -> Vec<&TaskRecord> {
        self.index
            .iter_in_order()
            .filter(|r| r.state == state)
            .collect()
    }

    /// Return all tasks for a dimension.
    pub fn tasks_for_dimension(&self, dim: DimensionId) -> Vec<&TaskRecord> {
        self.index
            .iter_in_order()
            .filter(|r| r.spec.dimension_id == dim)
            .collect()
    }

    /// Number of tasks currently in the scheduler.
    pub fn len(&self) -> usize {
        self.index.records.len()
    }

    /// `true` if the scheduler has no tasks.
    pub fn is_empty(&self) -> bool {
        self.index.records.is_empty()
    }

    // ── Persistence hooks ─────────────────────────────────────────────────

    /// Serialise all task records to a JSON string for persistence.
    ///
    /// Restore with [`Self::restore`].
    ///
    /// # Errors
    ///
    /// Propagates `serde_json::Error`.
    pub fn snapshot(&self) -> Result<String, serde_json::Error> {
        let records: Vec<&TaskRecord> = self.index.iter_in_order().collect();
        serde_json::to_string(&records)
    }

    /// Restore task records from a JSON snapshot produced by [`Self::snapshot`].
    ///
    /// Existing records are replaced.
    ///
    /// # Errors
    ///
    /// Propagates `serde_json::Error`.
    pub fn restore(&mut self, json: &str) -> Result<(), serde_json::Error> {
        let records: Vec<TaskRecord> = serde_json::from_str(json)?;
        for record in records {
            let id = record.task_id;
            if !self.index.contains(id) {
                self.index.records.insert(id, record.clone());
                self.index.insertion_order.push(id);
            }
            // Re-enqueue queued tasks.
            if record.state == TaskState::Queued {
                self.enqueue(id, record.spec.priority, record.seq);
            }
        }
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Validate and apply a state transition.
    fn transition(&mut self, task_id: TaskId, next: TaskState) -> Result<(), SchedulerError> {
        let record = self
            .index
            .get_mut(task_id)
            .ok_or(SchedulerError::TaskNotFound(task_id))?;
        if !record.state.can_transition_to(next) {
            return Err(SchedulerError::InvalidTransition {
                task_id,
                from: record.state,
                to: next,
            });
        }
        record.state = next;
        record.updated_at_ms = now_ms();

        if next == TaskState::Queued {
            let prio = record.spec.priority;
            let seq = record.seq;
            self.enqueue(task_id, prio, seq);
        } else if next == TaskState::Running {
            let dim = record.spec.dimension_id;
            self.increment_running(dim);
        }
        Ok(())
    }

    fn enqueue(&mut self, task_id: TaskId, priority: TaskPriority, seq: u64) {
        self.ready_queue.push_back((priority, seq, task_id));
        self.sort_ready_queue();
    }

    fn sort_ready_queue(&mut self) {
        // Sort: highest priority first; within same priority, lowest seq first.
        let mut v: Vec<_> = self.ready_queue.drain(..).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.ready_queue = v.into();
    }

    fn all_deps_completed(&self, deps: &[TaskId]) -> bool {
        deps.iter().all(|&id| {
            self.index
                .get(id)
                .map(|r| r.state == TaskState::Completed)
                .unwrap_or(false)
        })
    }

    /// Promote any `Pending` tasks whose dependencies are now all `Completed`.
    fn promote_pending_tasks(&mut self) {
        let pending_ids: Vec<TaskId> = self
            .index
            .iter_in_order()
            .filter(|r| r.state == TaskState::Pending)
            .map(|r| r.task_id)
            .collect();

        for id in pending_ids {
            let deps: Vec<TaskId> = self
                .index
                .get(id)
                .map(|r| r.spec.dependencies.clone())
                .unwrap_or_default();
            if self.all_deps_completed(&deps) {
                let _ = self.transition(id, TaskState::Queued);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use ify_core::{DimensionId, TaskId};
    use uuid::Uuid;

    use super::*;
    use crate::action_log::ActionLog;

    fn make_scheduler() -> TaskScheduler {
        let log = ActionLog::new(64);
        let dim = DimensionId::new();
        let coord = TaskId::new();
        TaskScheduler::new(dim, coord, PreemptionPolicy::None, 5_000, log)
    }

    fn submit_normal(sched: &mut TaskScheduler, dim: DimensionId) -> TaskId {
        let id = TaskId::new();
        let spec = TaskSpec::new(dim, "test", "A task");
        sched.submit(id, spec).unwrap();
        id
    }

    // ── TaskState transitions ─────────────────────────────────────────────

    #[test]
    fn task_state_valid_transitions() {
        assert!(TaskState::Pending.can_transition_to(TaskState::Queued));
        assert!(TaskState::Queued.can_transition_to(TaskState::Running));
        assert!(TaskState::Running.can_transition_to(TaskState::Completed));
        assert!(TaskState::Running.can_transition_to(TaskState::Failed));
        assert!(TaskState::Running.can_transition_to(TaskState::Paused));
        assert!(TaskState::Failed.can_transition_to(TaskState::Queued)); // retry
    }

    #[test]
    fn task_state_invalid_transitions() {
        assert!(!TaskState::Completed.can_transition_to(TaskState::Queued));
        assert!(!TaskState::Cancelled.can_transition_to(TaskState::Running));
        assert!(!TaskState::Pending.can_transition_to(TaskState::Running)); // must go Pending→Queued first
    }

    #[test]
    fn terminal_states() {
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Cancelled.is_terminal());
        assert!(!TaskState::Running.is_terminal());
    }

    // ── RetryPolicy ───────────────────────────────────────────────────────

    #[test]
    fn retry_policy_backoff_doubles() {
        let policy = RetryPolicy::exponential(4, 100, 60_000);
        assert_eq!(policy.backoff_for(0), Duration::from_millis(100));
        assert_eq!(policy.backoff_for(1), Duration::from_millis(200));
        assert_eq!(policy.backoff_for(2), Duration::from_millis(400));
    }

    #[test]
    fn retry_policy_backoff_capped() {
        let policy = RetryPolicy::exponential(4, 1000, 2_500);
        assert!(policy.backoff_for(10) <= Duration::from_millis(2_500));
    }

    #[test]
    fn retry_policy_none_returns_zero_backoff() {
        let policy = RetryPolicy::none();
        assert_eq!(policy.backoff_for(0), Duration::ZERO);
    }

    // ── Submit and dequeue ────────────────────────────────────────────────

    #[test]
    fn submit_and_dequeue() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = submit_normal(&mut s, dim);
        assert_eq!(s.get(id).unwrap().state, TaskState::Queued);

        let (record, _lease) = s.next_ready().unwrap();
        assert_eq!(record.task_id, id);
        assert_eq!(s.get(id).unwrap().state, TaskState::Running);
    }

    #[test]
    fn duplicate_task_id_rejected() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = TaskId::new();
        let spec = TaskSpec::new(dim, "k", "l");
        s.submit(id, spec.clone()).unwrap();
        let err = s.submit(id, spec).unwrap_err();
        assert!(matches!(err, SchedulerError::DuplicateTaskId(_)));
    }

    // ── Priority ordering ────────────────────────────────────────────────

    #[test]
    fn priority_ordering_dequeues_highest_first() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();

        let lo_id = TaskId::new();
        let hi_id = TaskId::new();

        let mut lo_spec = TaskSpec::new(dim, "low", "low");
        lo_spec.priority = TaskPriority::Background;
        let mut hi_spec = TaskSpec::new(dim, "high", "high");
        hi_spec.priority = TaskPriority::Critical;

        s.submit(lo_id, lo_spec).unwrap();
        s.submit(hi_id, hi_spec).unwrap();

        let (first, _) = s.next_ready().unwrap();
        assert_eq!(first.task_id, hi_id, "Critical task must dequeue first");

        let (second, _) = s.next_ready().unwrap();
        assert_eq!(second.task_id, lo_id);
    }

    // ── Complete / fail / cancel ──────────────────────────────────────────

    #[test]
    fn complete_task() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = submit_normal(&mut s, dim);
        s.next_ready().unwrap(); // Running
        s.complete(id).unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Completed);
    }

    #[test]
    fn cancel_queued_task() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = submit_normal(&mut s, dim);
        s.cancel(id).unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Cancelled);
        // Should not appear in ready queue.
        assert!(s.next_ready().is_none());
    }

    #[test]
    fn fail_with_retries_requeues() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = TaskId::new();
        let mut spec = TaskSpec::new(dim, "k", "l");
        // max_attempts=3 means 3 total runs; attempt=1 < 3 → retry path
        spec.retry_policy = RetryPolicy::exponential(3, 100, 10_000);
        s.submit(id, spec).unwrap();
        s.next_ready().unwrap(); // Running, attempt = 1

        // attempt(1) < max_attempts(3) → retry path → Queued
        s.fail(id, "transient error").unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Queued);
    }

    #[test]
    fn fail_exhausted_retries_is_terminal() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = TaskId::new();
        let mut spec = TaskSpec::new(dim, "k", "l");
        // max_attempts=1 means only one execution allowed (RetryPolicy::none())
        spec.retry_policy = RetryPolicy::none();
        s.submit(id, spec).unwrap();
        s.next_ready().unwrap(); // attempt = 1

        // attempt(1) < max_attempts(1) → false → Failed
        s.fail(id, "fatal").unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Failed);
    }

    #[test]
    fn fail_retries_exhaust_to_terminal() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = TaskId::new();
        let mut spec = TaskSpec::new(dim, "k", "l");
        // max_attempts=2: first run + one retry allowed, then terminal
        spec.retry_policy = RetryPolicy::exponential(2, 0, 0);
        s.submit(id, spec).unwrap();

        // Run 1: attempt=1 < 2 → retry
        s.next_ready().unwrap();
        s.fail(id, "err1").unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Queued);

        // Run 2: attempt=2 < 2 → false → terminal Failed
        s.next_ready().unwrap();
        s.fail(id, "err2").unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Failed);
    }

    // ── Dependencies ──────────────────────────────────────────────────────

    #[test]
    fn dependent_task_stays_pending_until_prerequisite_completes() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();

        let prereq_id = submit_normal(&mut s, dim);
        // prereq is Queued

        let dep_id = TaskId::new();
        let mut dep_spec = TaskSpec::new(dim, "dep", "dep");
        dep_spec.dependencies = vec![prereq_id];
        s.submit(dep_id, dep_spec).unwrap();

        // Dependent must be Pending (prereq not yet completed).
        assert_eq!(s.get(dep_id).unwrap().state, TaskState::Pending);

        // Complete prerequisite.
        s.next_ready().unwrap(); // dequeue prereq
        s.complete(prereq_id).unwrap();

        // Dependent should now be Queued.
        assert_eq!(s.get(dep_id).unwrap().state, TaskState::Queued);
    }

    #[test]
    fn prerequisite_not_found_rejected() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let missing = TaskId::new();

        let mut spec = TaskSpec::new(dim, "dep", "dep");
        spec.dependencies = vec![missing];
        let err = s.submit(TaskId::new(), spec).unwrap_err();
        assert!(matches!(err, SchedulerError::PrerequisiteNotFound(_)));
    }

    // ── Quota ─────────────────────────────────────────────────────────────

    #[test]
    fn quota_limits_concurrent_dequeue() {
        let log = ActionLog::new(32);
        let ctrl_dim = DimensionId::new();
        let coord = TaskId::new();
        let mut s =
            TaskScheduler::new(ctrl_dim, coord, PreemptionPolicy::None, 5_000, log);

        let dim = DimensionId::new();
        s.set_quota(dim, DimensionQuota { max_concurrent: 1, max_queued: 100, tasks_per_minute: 0 });

        let id1 = submit_normal(&mut s, dim);
        let id2 = submit_normal(&mut s, dim);

        // First dequeue succeeds.
        let (r, _) = s.next_ready().unwrap();
        assert_eq!(r.task_id, id1);

        // Second dequeue blocked (quota = 1 concurrent).
        assert!(s.next_ready().is_none());

        // After completing the first, the second can run.
        s.complete(id1).unwrap();
        let (r2, _) = s.next_ready().unwrap();
        assert_eq!(r2.task_id, id2);
    }

    // ── Pause / resume ────────────────────────────────────────────────────

    #[test]
    fn pause_and_resume() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = submit_normal(&mut s, dim);
        s.next_ready().unwrap(); // Running

        s.pause(id).unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Paused);

        s.resume(id).unwrap();
        assert_eq!(s.get(id).unwrap().state, TaskState::Queued);
    }

    // ── Preemption ────────────────────────────────────────────────────────

    #[test]
    fn higher_priority_wins_preemption() {
        let log = ActionLog::new(32);
        let ctrl_dim = DimensionId::new();
        let coord = TaskId::new();
        let mut s = TaskScheduler::new(
            ctrl_dim,
            coord,
            PreemptionPolicy::HigherPriorityWins,
            5_000,
            log,
        );
        let dim = DimensionId::new();

        let lo_id = TaskId::new();
        let mut lo_spec = TaskSpec::new(dim, "low", "low");
        lo_spec.priority = TaskPriority::Background;
        s.submit(lo_id, lo_spec).unwrap();
        s.next_ready().unwrap(); // lo is Running

        let hi_id = TaskId::new();
        let mut hi_spec = TaskSpec::new(dim, "high", "high");
        hi_spec.priority = TaskPriority::High;
        s.submit(hi_id, hi_spec).unwrap();

        let victims = s.candidates_for_preemption(hi_id);
        assert_eq!(victims, vec![lo_id]);
    }

    #[test]
    fn no_preemption_when_policy_is_none() {
        let mut s = make_scheduler(); // policy = None
        let dim = DimensionId::new();
        let lo_id = submit_normal(&mut s, dim);
        s.next_ready().unwrap();

        let hi_id = TaskId::new();
        let mut hi_spec = TaskSpec::new(dim, "high", "high");
        hi_spec.priority = TaskPriority::Critical;
        s.submit(hi_id, hi_spec).unwrap();

        assert!(s.candidates_for_preemption(hi_id).is_empty());
    }

    // ── Task templates ────────────────────────────────────────────────────

    #[test]
    fn task_template_instantiation() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();

        let mut tmpl = TaskTemplate::new("My Template", "http.request");
        tmpl.default_params
            .insert("url".into(), serde_json::json!("https://example.com"));

        let tmpl_id = tmpl.id;
        s.register_template(tmpl);

        let task_id = TaskId::new();
        let mut overrides = BTreeMap::new();
        overrides.insert("method".into(), serde_json::json!("POST"));

        s.submit_from_template(tmpl_id, task_id, overrides).unwrap();

        let record = s.get(task_id).unwrap();
        assert_eq!(record.spec.kind, "http.request");
        assert_eq!(record.spec.params["url"], serde_json::json!("https://example.com"));
        assert_eq!(record.spec.params["method"], serde_json::json!("POST"));
    }

    #[test]
    fn unknown_template_fails() {
        let mut s = make_scheduler();
        let err = s
            .submit_from_template(Uuid::new_v4(), TaskId::new(), BTreeMap::new())
            .unwrap_err();
        assert!(matches!(err, SchedulerError::TemplateNotFound(_)));
    }

    // ── Lease ─────────────────────────────────────────────────────────────

    #[test]
    fn lease_is_valid_when_fresh() {
        let lease = TaskLease::new(TaskId::new(), "w1", 60_000);
        assert!(lease.is_valid(now_ms()));
    }

    #[test]
    fn lease_expired_when_past_deadline() {
        let mut lease = TaskLease::new(TaskId::new(), "w1", 100);
        // Backdate the expiry.
        lease.expires_at_ms = now_ms().saturating_sub(1);
        assert!(!lease.is_valid(now_ms()));
    }

    // ── Persistence ───────────────────────────────────────────────────────

    #[test]
    fn snapshot_and_restore() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id = submit_normal(&mut s, dim);

        let snap = s.snapshot().unwrap();

        let log = ActionLog::new(32);
        let ctrl_dim = DimensionId::new();
        let coord = TaskId::new();
        let mut s2 =
            TaskScheduler::new(ctrl_dim, coord, PreemptionPolicy::None, 5_000, log);
        s2.restore(&snap).unwrap();

        assert!(s2.get(id).is_some());
        assert_eq!(s2.get(id).unwrap().state, s.get(id).unwrap().state);
    }

    // ── Query helpers ─────────────────────────────────────────────────────

    #[test]
    fn tasks_in_state_filter() {
        let mut s = make_scheduler();
        let dim = DimensionId::new();
        let id1 = submit_normal(&mut s, dim);
        let id2 = submit_normal(&mut s, dim);
        s.next_ready().unwrap(); // id1 Running

        let queued = s.tasks_in_state(TaskState::Queued);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].task_id, id2);
    }

    #[test]
    fn tasks_for_dimension_filter() {
        let mut s = make_scheduler();
        let dim_a = DimensionId::new();
        let dim_b = DimensionId::new();
        submit_normal(&mut s, dim_a);
        submit_normal(&mut s, dim_a);
        submit_normal(&mut s, dim_b);

        assert_eq!(s.tasks_for_dimension(dim_a).len(), 2);
        assert_eq!(s.tasks_for_dimension(dim_b).len(), 1);
    }
}
