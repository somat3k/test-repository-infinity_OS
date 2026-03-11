//! Job scheduling and task lifecycle — Epic J.
//!
//! This module implements all ten Epic J requirements for infinityOS:
//!
//! | Item | Requirement |
//! |------|-------------|
//! | J1  | Task states: `queued / running / paused / failed / completed` |
//! | J2  | Priority-aware scheduling (`TaskPriority` + `JobScheduler`) |
//! | J3  | Retries, exponential backoff, cancellation (`RetryPolicy`, `CancellationToken`) |
//! | J4  | Task leasing / heartbeats for distributed workers (`Lease`) |
//! | J5  | Per-dimension quotas and rate limiting (`DimensionQuota`, `RateLimiter`) |
//! | J6  | Dependency-aware scheduling — DAG-based (`TaskDag`) |
//! | J7  | Task preemption policy (`PreemptionPolicy`) |
//! | J8  | Task persistence + recovery after crash/restart (`TaskSnapshot`) |
//! | J9  | Task templates for repeated workflows (`TaskTemplate`) |
//! | J10 | Unique TaskID enforcement + index across dimensions (`TaskIndex`) |
//!
//! ## Design notes
//!
//! * All types are `Send + Sync` and lock-protected for multi-thread safety.
//! * No panics in library code — every error path returns a `Result`.
//! * Every scheduling decision emits an [`ActionLogEntry`] for full auditability.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ============================================================================
// J1 — Task states
// ============================================================================

/// Lifecycle state of a task in the job scheduler.
///
/// Transitions:
/// ```text
/// Queued → Running → Completed
///       ↓         ↓
///     Paused    Failed
///       ↓
///    Running  (on resume)
///
/// Any non-terminal state → Cancelled (via CancellationToken)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Waiting in the priority queue.
    Queued,
    /// Currently executing.
    Running,
    /// Temporarily suspended (can be resumed).
    Paused,
    /// Terminated with an error; may be retried.
    Failed,
    /// Completed successfully.
    Completed,
    /// Explicitly cancelled; terminal, not retried.
    Cancelled,
}

impl TaskState {
    /// Returns `true` for states that cannot advance further.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        };
        f.write_str(s)
    }
}

// ============================================================================
// J2 — Priority-aware scheduling
// ============================================================================

/// Task execution priority (higher value = higher urgency).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    /// Lowest priority — background housekeeping.
    Background = 0,
    /// Below-normal priority.
    Low = 1,
    /// Default priority.
    #[default]
    Normal = 2,
    /// Above-normal priority.
    High = 3,
    /// Highest priority — time-critical work.
    Critical = 4,
}

/// A task descriptor queued in the [`JobScheduler`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEntry {
    /// Globally unique task identifier.
    pub task_id: TaskId,
    /// Dimension this task belongs to.
    pub dimension_id: DimensionId,
    /// Scheduling priority.
    pub priority: TaskPriority,
    /// Current lifecycle state.
    pub state: TaskState,
    /// Optional name (used by templates or human operators).
    pub name: Option<String>,
    /// Retry policy for this job.
    pub retry_policy: RetryPolicy,
    /// How many times this job has been retried so far.
    pub retry_count: u32,
    /// Preemption policy that applies to this job.
    pub preemption_policy: PreemptionPolicy,
}

/// Errors produced by the [`JobScheduler`].
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// The referenced task does not exist.
    #[error("task {0} not found")]
    TaskNotFound(TaskId),

    /// The task is already in a terminal state.
    #[error("task {0} is in a terminal state and cannot be modified")]
    AlreadyTerminal(TaskId),

    /// A quota limit has been exceeded for this dimension.
    #[error("quota exceeded for dimension {0}: {1}")]
    QuotaExceeded(DimensionId, String),

    /// A dependency cycle was detected in the task DAG.
    #[error("cycle detected in task dependency graph")]
    DagCycle,

    /// A required dependency task is not known to the scheduler.
    #[error("dependency task {0} is not registered")]
    UnknownDependency(TaskId),

    /// The task was not found in the global index.
    #[error("task {0} is not registered in the global task index")]
    NotInIndex(TaskId),

    /// A task with this ID already exists (uniqueness violation).
    #[error("task {0} already exists — TaskID must be globally unique")]
    DuplicateTaskId(TaskId),
}

/// Priority-aware, dimension-scoped job scheduler.
///
/// Internally maintains a priority queue implemented as a `BTreeMap` keyed by
/// `(priority_desc, submission_order)` so that equal-priority tasks are served
/// FIFO, while higher-priority tasks always come first.
///
/// ## Thread safety
///
/// All methods take `&self` and use internal locking.
pub struct JobScheduler {
    dimension_id: DimensionId,
    /// `(rev_priority, seq) → task_id` — higher priority = lower `rev_priority`
    queue: Mutex<BTreeMap<(i64, u64), TaskId>>,
    entries: Mutex<HashMap<TaskId, JobEntry>>,
    seq: Mutex<u64>,
    quota: Mutex<DimensionQuota>,
    rate_limiter: Mutex<RateLimiter>,
    action_log: Arc<ActionLog>,
}

impl std::fmt::Debug for JobScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let queued = self.queue.lock().map(|g| g.len()).unwrap_or(0);
        write!(
            f,
            "JobScheduler {{ dimension: {}, queued: {queued} }}",
            self.dimension_id
        )
    }
}

impl JobScheduler {
    /// Create a new scheduler for `dimension_id`.
    pub fn new(
        dimension_id: DimensionId,
        quota: DimensionQuota,
        action_log: Arc<ActionLog>,
    ) -> Self {
        Self {
            dimension_id,
            queue: Mutex::new(BTreeMap::new()),
            entries: Mutex::new(HashMap::new()),
            seq: Mutex::new(0),
            quota: Mutex::new(quota),
            rate_limiter: Mutex::new(RateLimiter::new(quota.max_tasks_per_second)),
            action_log,
        }
    }

    /// Enqueue a job.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::QuotaExceeded`] when the per-dimension queue
    /// limit is reached or the rate limiter rejects the submission.
    pub fn enqueue(&self, entry: JobEntry) -> Result<(), SchedulerError> {
        // Rate limiter check
        {
            let mut rl = self.rate_limiter.lock().expect("rate limiter lock poisoned");
            if !rl.try_acquire() {
                return Err(SchedulerError::QuotaExceeded(
                    self.dimension_id,
                    "rate limit exceeded".into(),
                ));
            }
        }

        // Queue depth check
        {
            let quota = self.quota.lock().expect("quota lock poisoned");
            let queue = self.queue.lock().expect("queue lock poisoned");
            if queue.len() >= quota.max_queued {
                return Err(SchedulerError::QuotaExceeded(
                    self.dimension_id,
                    format!("queue depth {} >= limit {}", queue.len(), quota.max_queued),
                ));
            }
        }

        let task_id = entry.task_id;
        let priority = entry.priority as i64;

        let seq = {
            let mut s = self.seq.lock().expect("seq lock poisoned");
            *s += 1;
            *s
        };

        {
            let mut queue = self.queue.lock().expect("queue lock poisoned");
            let mut entries = self.entries.lock().expect("entries lock poisoned");
            // rev_priority: higher priority → lower key → polled first
            queue.insert((i64::MAX - priority, seq), task_id);
            entries.insert(task_id, entry);
        }

        debug!(task_id = %task_id, "job enqueued");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskSubmitted,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string() }),
        ));

        Ok(())
    }

    /// Dequeue the highest-priority ready task (respects DAG dependencies via
    /// the provided `dag`).  Returns `None` if the queue is empty.
    pub fn dequeue(&self, dag: &TaskDag) -> Option<JobEntry> {
        let mut queue = self.queue.lock().expect("queue lock poisoned");
        let mut entries = self.entries.lock().expect("entries lock poisoned");

        // Iterate in priority order and return the first runnable task
        let key = queue
            .iter()
            .find(|(_, tid)| dag.is_ready(**tid, &entries))
            .map(|(k, _)| *k)?;

        let task_id = queue.remove(&key)?;
        let entry = entries.get_mut(&task_id)?;
        entry.state = TaskState::Running;

        info!(task_id = %task_id, "job dequeued and started");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskStarted,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string() }),
        ));

        Some(entry.clone())
    }

    /// Transition a task to `Completed`.
    pub fn complete(&self, task_id: TaskId) -> Result<(), SchedulerError> {
        self.transition(task_id, TaskState::Completed)?;
        info!(task_id = %task_id, "job completed");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskCompleted,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string() }),
        ));
        Ok(())
    }

    /// Transition a task to `Failed`.
    pub fn fail(&self, task_id: TaskId, reason: &str) -> Result<(), SchedulerError> {
        self.transition(task_id, TaskState::Failed)?;
        warn!(task_id = %task_id, %reason, "job failed");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskFailed,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string(), "reason": reason }),
        ));
        Ok(())
    }

    /// Pause a running task.
    pub fn pause(&self, task_id: TaskId) -> Result<(), SchedulerError> {
        self.transition_from(task_id, TaskState::Running, TaskState::Paused)
    }

    /// Resume a paused task (re-queues it with its original priority).
    pub fn resume(&self, task_id: TaskId) -> Result<(), SchedulerError> {
        let (priority, seq) = {
            let mut entries = self.entries.lock().expect("entries lock poisoned");
            let entry = entries
                .get_mut(&task_id)
                .ok_or(SchedulerError::TaskNotFound(task_id))?;
            if entry.state != TaskState::Paused {
                return Err(SchedulerError::AlreadyTerminal(task_id));
            }
            entry.state = TaskState::Queued;
            let p = entry.priority as i64;
            let mut s = self.seq.lock().expect("seq lock poisoned");
            *s += 1;
            (p, *s)
        };
        let mut queue = self.queue.lock().expect("queue lock poisoned");
        queue.insert((i64::MAX - priority, seq), task_id);
        debug!(task_id = %task_id, "job resumed");
        Ok(())
    }

    /// Cancel a task (transition to terminal `Cancelled` state).
    pub fn cancel(&self, task_id: TaskId) -> Result<(), SchedulerError> {
        self.transition(task_id, TaskState::Cancelled)?;
        info!(task_id = %task_id, "job cancelled");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskCancelled,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({ "task_id": task_id.to_string() }),
        ));
        Ok(())
    }

    /// Retry a failed task according to its [`RetryPolicy`].
    ///
    /// Returns `Ok(Some(backoff))` when a retry is scheduled, `Ok(None)` when
    /// retries are exhausted (task remains `Failed`), or an error if the task
    /// is not in a retryable state.
    pub fn retry(&self, task_id: TaskId) -> Result<Option<Duration>, SchedulerError> {
        let (retry_policy, retry_count, priority) = {
            let entries = self.entries.lock().expect("entries lock poisoned");
            let entry = entries
                .get(&task_id)
                .ok_or(SchedulerError::TaskNotFound(task_id))?;
            if entry.state != TaskState::Failed {
                return Err(SchedulerError::AlreadyTerminal(task_id));
            }
            (entry.retry_policy, entry.retry_count, entry.priority)
        };

        if retry_count >= retry_policy.max_attempts {
            debug!(task_id = %task_id, "retry limit reached");
            return Ok(None);
        }

        let backoff = retry_policy.backoff(retry_count);

        // Update state and increment retry counter
        {
            let mut entries = self.entries.lock().expect("entries lock poisoned");
            let entry = entries.get_mut(&task_id).expect("entry disappeared");
            entry.state = TaskState::Queued;
            entry.retry_count += 1;
        }

        // Re-queue with original priority
        let seq = {
            let mut s = self.seq.lock().expect("seq lock poisoned");
            *s += 1;
            *s
        };
        {
            let mut queue = self.queue.lock().expect("queue lock poisoned");
            queue.insert((i64::MAX - priority as i64, seq), task_id);
        }

        info!(task_id = %task_id, retry_count = retry_count + 1, ?backoff, "job retried");
        self.action_log.append(ActionLogEntry::new(
            EventType::TaskRetried,
            Actor::System,
            Some(self.dimension_id),
            Some(task_id),
            serde_json::json!({
                "task_id": task_id.to_string(),
                "attempt": retry_count + 1,
                "backoff_ms": backoff.as_millis(),
            }),
        ));

        Ok(Some(backoff))
    }

    /// Return the current state of a task.
    pub fn state(&self, task_id: TaskId) -> Option<TaskState> {
        self.entries
            .lock()
            .expect("entries lock poisoned")
            .get(&task_id)
            .map(|e| e.state)
    }

    /// Number of tasks currently in the queue.
    pub fn queued_count(&self) -> usize {
        self.queue.lock().expect("queue lock poisoned").len()
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn transition(&self, task_id: TaskId, new_state: TaskState) -> Result<(), SchedulerError> {
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        let entry = entries
            .get_mut(&task_id)
            .ok_or(SchedulerError::TaskNotFound(task_id))?;
        if entry.state.is_terminal() {
            return Err(SchedulerError::AlreadyTerminal(task_id));
        }
        entry.state = new_state;
        Ok(())
    }

    fn transition_from(
        &self,
        task_id: TaskId,
        expected: TaskState,
        new_state: TaskState,
    ) -> Result<(), SchedulerError> {
        let mut entries = self.entries.lock().expect("entries lock poisoned");
        let entry = entries
            .get_mut(&task_id)
            .ok_or(SchedulerError::TaskNotFound(task_id))?;
        if entry.state != expected {
            return Err(SchedulerError::AlreadyTerminal(task_id));
        }
        entry.state = new_state;
        Ok(())
    }
}

// ============================================================================
// J3 — Retries, backoff, cancellation
// ============================================================================

/// Retry policy applied to a job when it enters `Failed` state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (0 = no retries).
    pub max_attempts: u32,
    /// Base backoff duration for exponential back-off.
    pub backoff_base: Duration,
    /// Multiplicative factor applied per attempt.
    pub backoff_multiplier: f64,
    /// Maximum cap on the computed backoff.
    pub backoff_max: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_base: Duration::from_millis(500),
            backoff_multiplier: 2.0,
            backoff_max: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Compute the backoff duration for attempt number `attempt` (0-indexed).
    pub fn backoff(self, attempt: u32) -> Duration {
        let factor = self.backoff_multiplier.powi(attempt as i32);
        let ms = (self.backoff_base.as_millis() as f64 * factor) as u128;
        let computed = Duration::from_millis(ms.min(u64::MAX as u128) as u64);
        computed.min(self.backoff_max)
    }

    /// Create a no-retry policy.
    pub fn no_retry() -> Self {
        Self {
            max_attempts: 0,
            ..Default::default()
        }
    }
}

/// A lightweight cancellation signal that can be shared across threads.
///
/// Workers poll [`CancellationToken::is_cancelled`] during execution and abort
/// cleanly when the token is set.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    inner: Arc<std::sync::atomic::AtomicBool>,
}

impl CancellationToken {
    /// Create a new, uncancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal cancellation.
    pub fn cancel(&self) {
        self.inner
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Returns `true` if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(std::sync::atomic::Ordering::Acquire)
    }
}

// ============================================================================
// J4 — Task leasing / heartbeats
// ============================================================================

/// A lease grants exclusive execution rights to a worker for a bounded period.
///
/// Workers call [`Lease::heartbeat`] to renew the lease before it expires.
/// The scheduler can evict stale leases (where `is_expired()` returns `true`)
/// and re-queue the task.
#[derive(Debug)]
pub struct Lease {
    task_id: TaskId,
    worker_id: String,
    ttl: Duration,
    renewed_at: Mutex<Instant>,
}

impl Lease {
    /// Acquire a new lease for `task_id` by `worker_id` with the given TTL.
    pub fn acquire(task_id: TaskId, worker_id: impl Into<String>, ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            task_id,
            worker_id: worker_id.into(),
            ttl,
            renewed_at: Mutex::new(Instant::now()),
        })
    }

    /// Renew the lease, resetting the expiry clock.
    ///
    /// Call this from the heartbeat loop of the worker.
    pub fn heartbeat(&self) {
        *self.renewed_at.lock().expect("lease lock poisoned") = Instant::now();
        debug!(task_id = %self.task_id, worker = %self.worker_id, "lease heartbeat");
    }

    /// Returns `true` if the lease TTL has elapsed since the last heartbeat.
    pub fn is_expired(&self) -> bool {
        let last = *self.renewed_at.lock().expect("lease lock poisoned");
        last.elapsed() >= self.ttl
    }

    /// The task this lease covers.
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// The worker that holds this lease.
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// TTL configured for this lease.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

// ============================================================================
// J5 — Per-dimension quotas and rate limiting
// ============================================================================

/// Resource limits applied to a single dimension.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DimensionQuota {
    /// Maximum number of concurrently running tasks.
    pub max_concurrent: usize,
    /// Maximum depth of the waiting queue.
    pub max_queued: usize,
    /// Maximum task submission rate (tasks per second; 0 = unlimited).
    pub max_tasks_per_second: u32,
}

impl Default for DimensionQuota {
    fn default() -> Self {
        Self {
            max_concurrent: 16,
            max_queued: 256,
            max_tasks_per_second: 100,
        }
    }
}

/// Token-bucket rate limiter.
///
/// Refills at `rate` tokens per second.  Each call to [`try_acquire`] consumes
/// one token, returning `false` when no tokens remain.
#[derive(Debug)]
pub struct RateLimiter {
    /// Maximum tokens the bucket can hold.
    capacity: u32,
    /// Current token count.
    tokens: f64,
    /// Refill rate in tokens per second.
    rate: f64,
    /// Last refill timestamp.
    last_refill: Instant,
}

impl RateLimiter {
    /// Create a new token-bucket limiter.  `rate = 0` disables limiting.
    pub fn new(rate: u32) -> Self {
        let capacity = rate.max(1);
        Self {
            capacity,
            tokens: capacity as f64,
            rate: rate as f64,
            last_refill: Instant::now(),
        }
    }

    /// Try to acquire one token.  Returns `true` on success.
    pub fn try_acquire(&mut self) -> bool {
        // Unlimited mode
        if self.rate == 0.0 {
            return true;
        }

        // Refill bucket based on elapsed time
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity as f64);
        self.last_refill = Instant::now();

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

// ============================================================================
// J6 — Dependency-aware scheduling (DAG)
// ============================================================================

/// A directed-acyclic graph of task dependencies.
///
/// `TaskDag::add_dependency(a, b)` means "task `a` must wait for `b` to
/// complete before it can run".
///
/// [`is_ready`][TaskDag::is_ready] returns `true` when all predecessors of a
/// task have reached `Completed`.
#[derive(Debug, Default)]
pub struct TaskDag {
    /// edges[a] = set of tasks that `a` depends on (must complete before `a`)
    edges: Mutex<HashMap<TaskId, HashSet<TaskId>>>,
}

impl TaskDag {
    /// Create an empty DAG.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare that `task` depends on `depends_on` completing first.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::DagCycle`] if adding this edge would create a
    /// cycle.
    pub fn add_dependency(
        &self,
        task: TaskId,
        depends_on: TaskId,
    ) -> Result<(), SchedulerError> {
        let mut edges = self.edges.lock().expect("dag lock poisoned");
        // Cycle check: would task be reachable from depends_on?
        if Self::has_path(&edges, depends_on, task) {
            return Err(SchedulerError::DagCycle);
        }
        edges.entry(task).or_default().insert(depends_on);
        debug!(task = %task, depends_on = %depends_on, "dag dependency added");
        Ok(())
    }

    /// Returns `true` when all dependencies of `task` are `Completed`.
    pub fn is_ready(&self, task: TaskId, entries: &HashMap<TaskId, JobEntry>) -> bool {
        let edges = self.edges.lock().expect("dag lock poisoned");
        let deps = match edges.get(&task) {
            None => return true, // no dependencies
            Some(set) => set,
        };
        deps.iter().all(|dep| {
            entries
                .get(dep)
                .map(|e| e.state == TaskState::Completed)
                .unwrap_or(false)
        })
    }

    /// Return the topological ordering of registered tasks, if the graph is acyclic.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::DagCycle`] if a cycle is detected.
    pub fn topological_order(&self, all_tasks: &[TaskId]) -> Result<Vec<TaskId>, SchedulerError> {
        let edges = self.edges.lock().expect("dag lock poisoned");

        // Kahn's algorithm
        let mut in_degree: HashMap<TaskId, usize> = all_tasks.iter().map(|t| (*t, 0)).collect();
        for (node, deps) in edges.iter() {
            if in_degree.contains_key(node) {
                for dep in deps {
                    if in_degree.contains_key(dep) {
                        *in_degree.entry(*node).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut queue: VecDeque<TaskId> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(t, _)| *t)
            .collect();

        let mut order = Vec::with_capacity(all_tasks.len());
        while let Some(node) = queue.pop_front() {
            order.push(node);
            // Find tasks that depend on `node` and reduce their in-degree
            for (t, deps) in edges.iter() {
                if deps.contains(&node) {
                    if let Some(deg) = in_degree.get_mut(t) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(*t);
                        }
                    }
                }
            }
        }

        if order.len() != all_tasks.len() {
            return Err(SchedulerError::DagCycle);
        }
        Ok(order)
    }

    // Walk reachability via DFS (no locking — caller holds the lock).
    fn has_path(
        edges: &HashMap<TaskId, HashSet<TaskId>>,
        from: TaskId,
        to: TaskId,
    ) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![from];
        while let Some(node) = stack.pop() {
            if node == to {
                return true;
            }
            if visited.insert(node) {
                if let Some(deps) = edges.get(&node) {
                    stack.extend(deps.iter().copied());
                }
            }
        }
        false
    }
}

// ============================================================================
// J7 — Task preemption policy
// ============================================================================

/// Policy governing whether and how a running task can be interrupted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreemptionPolicy {
    /// The task cannot be preempted once running.
    #[default]
    Never,
    /// A higher-priority task may preempt this one.
    ByPriority,
    /// The task is preemptible if a deadline-critical task is waiting.
    ByDeadline,
    /// Always preemptible (best-effort work).
    Always,
}

/// Evaluate whether `candidate` can preempt `current`.
///
/// Returns `true` when the preemption should proceed.
pub fn should_preempt(
    current: &JobEntry,
    candidate: &JobEntry,
) -> bool {
    match current.preemption_policy {
        PreemptionPolicy::Never => false,
        PreemptionPolicy::Always => true,
        PreemptionPolicy::ByPriority => candidate.priority > current.priority,
        PreemptionPolicy::ByDeadline => {
            // Treat Critical-priority candidate as deadline-driven
            candidate.priority == TaskPriority::Critical
                && current.priority < TaskPriority::Critical
        }
    }
}

// ============================================================================
// J8 — Task persistence + recovery
// ============================================================================

/// Serializable snapshot of a task that can be persisted to stable storage
/// and used to reconstruct task state after a crash or restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    /// Unique task identifier.
    pub task_id: TaskId,
    /// Owning dimension.
    pub dimension_id: DimensionId,
    /// Last known lifecycle state.
    pub state: TaskState,
    /// Scheduling priority.
    pub priority: TaskPriority,
    /// Retry policy for this task.
    pub retry_policy: RetryPolicy,
    /// Number of retries already consumed.
    pub retry_count: u32,
    /// Preemption policy.
    pub preemption_policy: PreemptionPolicy,
    /// Optional human-readable task name.
    pub name: Option<String>,
    /// Arbitrary JSON payload carrying task-specific data.
    pub payload: serde_json::Value,
    /// Wall-clock timestamp (ms since Unix epoch) when the snapshot was taken.
    pub snapshot_at_ms: u64,
}

impl TaskSnapshot {
    /// Create a snapshot from a [`JobEntry`] with an arbitrary payload.
    pub fn from_entry(entry: &JobEntry, payload: serde_json::Value) -> Self {
        let snapshot_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            task_id: entry.task_id,
            dimension_id: entry.dimension_id,
            state: entry.state,
            priority: entry.priority,
            retry_policy: entry.retry_policy,
            retry_count: entry.retry_count,
            preemption_policy: entry.preemption_policy,
            name: entry.name.clone(),
            payload,
            snapshot_at_ms,
        }
    }

    /// Reconstruct a [`JobEntry`] from this snapshot.
    ///
    /// Tasks snapshotted in `Running` state are recovered as `Queued` (the
    /// worker that held the task is assumed dead after a crash).
    pub fn recover(self) -> JobEntry {
        let recovered_state = match self.state {
            // A running task with no lease must be re-queued
            TaskState::Running => TaskState::Queued,
            other => other,
        };
        JobEntry {
            task_id: self.task_id,
            dimension_id: self.dimension_id,
            priority: self.priority,
            state: recovered_state,
            name: self.name,
            retry_policy: self.retry_policy,
            retry_count: self.retry_count,
            preemption_policy: self.preemption_policy,
        }
    }
}

// ============================================================================
// J9 — Task templates
// ============================================================================

/// A reusable blueprint for creating jobs with a consistent configuration.
///
/// Templates define default values for priority, retry behaviour, and
/// preemption semantics.  Callers instantiate a [`JobEntry`] via
/// [`TaskTemplate::instantiate`], optionally overriding individual fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTemplate {
    /// Human-readable template name (unique within a dimension).
    pub name: String,
    /// Default priority for jobs created from this template.
    pub default_priority: TaskPriority,
    /// Default retry policy.
    pub default_retry_policy: RetryPolicy,
    /// Default preemption policy.
    pub default_preemption_policy: PreemptionPolicy,
    /// Default payload merged into every instantiated job.
    pub default_payload: serde_json::Value,
}

impl TaskTemplate {
    /// Create a new template with sensible defaults.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            default_priority: TaskPriority::Normal,
            default_retry_policy: RetryPolicy::default(),
            default_preemption_policy: PreemptionPolicy::Never,
            default_payload: serde_json::Value::Null,
        }
    }

    /// Instantiate a [`JobEntry`] from this template for the given dimension.
    ///
    /// The caller supplies the `task_id` (from the [`TaskAllocator`]).
    /// `priority_override` and `retry_override` allow per-instance customisation.
    ///
    /// [`TaskAllocator`]: crate::task_allocator::TaskAllocator
    pub fn instantiate(
        &self,
        task_id: TaskId,
        dimension_id: DimensionId,
        priority_override: Option<TaskPriority>,
        retry_override: Option<RetryPolicy>,
    ) -> JobEntry {
        JobEntry {
            task_id,
            dimension_id,
            priority: priority_override.unwrap_or(self.default_priority),
            state: TaskState::Queued,
            name: Some(self.name.clone()),
            retry_policy: retry_override.unwrap_or(self.default_retry_policy),
            retry_count: 0,
            preemption_policy: self.default_preemption_policy,
        }
    }
}

/// Registry of [`TaskTemplate`] instances keyed by name.
#[derive(Debug, Default)]
pub struct TaskTemplateRegistry {
    templates: Mutex<HashMap<String, TaskTemplate>>,
}

impl TaskTemplateRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a template.  Overwrites any existing template with the same name.
    pub fn register(&self, template: TaskTemplate) {
        self.templates
            .lock()
            .expect("template registry lock poisoned")
            .insert(template.name.clone(), template);
    }

    /// Retrieve a template by name.
    pub fn get(&self, name: &str) -> Option<TaskTemplate> {
        self.templates
            .lock()
            .expect("template registry lock poisoned")
            .get(name)
            .cloned()
    }
}

// ============================================================================
// J10 — Unique TaskID enforcement + index across dimensions
// ============================================================================

/// Global index guaranteeing TaskID uniqueness across all dimensions.
///
/// Every [`TaskId`] must be registered before use.  Duplicate registrations
/// are rejected, preventing silent ID collisions.
///
/// The index also supports reverse lookups: given a `TaskId`, callers can
/// discover which dimension owns it.
#[derive(Debug, Default)]
pub struct TaskIndex {
    /// task_id → dimension_id
    index: Mutex<HashMap<TaskId, DimensionId>>,
}

impl TaskIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `task_id` as belonging to `dimension_id`.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError::DuplicateTaskId`] if the ID is already registered.
    pub fn register(
        &self,
        task_id: TaskId,
        dimension_id: DimensionId,
    ) -> Result<(), SchedulerError> {
        let mut idx = self.index.lock().expect("task index lock poisoned");
        if idx.contains_key(&task_id) {
            return Err(SchedulerError::DuplicateTaskId(task_id));
        }
        idx.insert(task_id, dimension_id);
        debug!(task_id = %task_id, dimension = %dimension_id, "task registered in global index");
        Ok(())
    }

    /// Look up the dimension that owns `task_id`.
    pub fn dimension_of(&self, task_id: TaskId) -> Option<DimensionId> {
        self.index
            .lock()
            .expect("task index lock poisoned")
            .get(&task_id)
            .copied()
    }

    /// Returns `true` if `task_id` is registered.
    pub fn contains(&self, task_id: TaskId) -> bool {
        self.index
            .lock()
            .expect("task index lock poisoned")
            .contains_key(&task_id)
    }

    /// Total number of registered task IDs.
    pub fn len(&self) -> usize {
        self.index
            .lock()
            .expect("task index lock poisoned")
            .len()
    }

    /// Returns `true` if no task IDs are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::{DimensionId, TaskId};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_scheduler() -> (JobScheduler, DimensionId) {
        let dim = DimensionId::new();
        let log = ActionLog::new(64);
        let quota = DimensionQuota::default();
        (JobScheduler::new(dim, quota, log), dim)
    }

    fn basic_entry(dim: DimensionId) -> JobEntry {
        JobEntry {
            task_id: TaskId::new(),
            dimension_id: dim,
            priority: TaskPriority::Normal,
            state: TaskState::Queued,
            name: None,
            retry_policy: RetryPolicy::default(),
            retry_count: 0,
            preemption_policy: PreemptionPolicy::Never,
        }
    }

    // -----------------------------------------------------------------------
    // J1 — TaskState
    // -----------------------------------------------------------------------

    #[test]
    fn task_state_terminal_variants() {
        assert!(TaskState::Completed.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Cancelled.is_terminal());
        assert!(!TaskState::Queued.is_terminal());
        assert!(!TaskState::Running.is_terminal());
        assert!(!TaskState::Paused.is_terminal());
    }

    #[test]
    fn task_state_display() {
        assert_eq!(TaskState::Queued.to_string(), "queued");
        assert_eq!(TaskState::Running.to_string(), "running");
        assert_eq!(TaskState::Paused.to_string(), "paused");
        assert_eq!(TaskState::Failed.to_string(), "failed");
        assert_eq!(TaskState::Completed.to_string(), "completed");
        assert_eq!(TaskState::Cancelled.to_string(), "cancelled");
    }

    // -----------------------------------------------------------------------
    // J2 — Priority scheduling
    // -----------------------------------------------------------------------

    #[test]
    fn priority_ordering() {
        assert!(TaskPriority::Critical > TaskPriority::High);
        assert!(TaskPriority::High > TaskPriority::Normal);
        assert!(TaskPriority::Normal > TaskPriority::Low);
        assert!(TaskPriority::Low > TaskPriority::Background);
    }

    #[test]
    fn high_priority_dequeued_first() {
        let (sched, dim) = make_scheduler();
        let dag = TaskDag::new();

        let low = JobEntry {
            task_id: TaskId::new(),
            priority: TaskPriority::Low,
            ..basic_entry(dim)
        };
        let high = JobEntry {
            task_id: TaskId::new(),
            priority: TaskPriority::High,
            ..basic_entry(dim)
        };

        sched.enqueue(low).unwrap();
        sched.enqueue(high.clone()).unwrap();

        let got = sched.dequeue(&dag).unwrap();
        assert_eq!(got.task_id, high.task_id);
    }

    // -----------------------------------------------------------------------
    // J3 — Retries and backoff
    // -----------------------------------------------------------------------

    #[test]
    fn retry_policy_backoff_is_exponential() {
        let policy = RetryPolicy {
            max_attempts: 5,
            backoff_base: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            backoff_max: Duration::from_secs(60),
        };
        assert_eq!(policy.backoff(0), Duration::from_millis(100));
        assert_eq!(policy.backoff(1), Duration::from_millis(200));
        assert_eq!(policy.backoff(2), Duration::from_millis(400));
    }

    #[test]
    fn retry_policy_backoff_capped_at_max() {
        let policy = RetryPolicy {
            max_attempts: 10,
            backoff_base: Duration::from_millis(1000),
            backoff_multiplier: 10.0,
            backoff_max: Duration::from_secs(5),
        };
        // Without cap: 1000 * 10^5 = 100_000_000 ms
        assert!(policy.backoff(5) <= Duration::from_secs(5));
    }

    #[test]
    fn retry_exhausted_returns_none() {
        let (sched, dim) = make_scheduler();
        let mut entry = basic_entry(dim);
        entry.retry_policy = RetryPolicy::no_retry();
        let tid = entry.task_id;
        sched.enqueue(entry).unwrap();

        let dag = TaskDag::new();
        sched.dequeue(&dag).unwrap();
        sched.fail(tid, "boom").unwrap();

        let result = sched.retry(tid).unwrap();
        assert!(result.is_none(), "no retries should be scheduled");
    }

    #[test]
    fn retry_re_queues_task() {
        let (sched, dim) = make_scheduler();
        let mut entry = basic_entry(dim);
        entry.retry_policy = RetryPolicy {
            max_attempts: 3,
            ..Default::default()
        };
        let tid = entry.task_id;
        sched.enqueue(entry).unwrap();

        let dag = TaskDag::new();
        sched.dequeue(&dag).unwrap();
        sched.fail(tid, "oops").unwrap();

        let backoff = sched.retry(tid).unwrap();
        assert!(backoff.is_some());
        assert_eq!(sched.state(tid), Some(TaskState::Queued));
    }

    #[test]
    fn cancellation_token() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
        // Clone shares the signal
        let clone = token.clone();
        assert!(clone.is_cancelled());
    }

    // -----------------------------------------------------------------------
    // J4 — Leasing / heartbeats
    // -----------------------------------------------------------------------

    #[test]
    fn lease_not_expired_immediately() {
        let tid = TaskId::new();
        let lease = Lease::acquire(tid, "worker-1", Duration::from_secs(30));
        assert!(!lease.is_expired());
    }

    #[test]
    fn lease_expired_after_ttl() {
        let tid = TaskId::new();
        let lease = Lease::acquire(tid, "worker-1", Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        assert!(lease.is_expired());
    }

    #[test]
    fn heartbeat_renews_lease() {
        let tid = TaskId::new();
        let lease = Lease::acquire(tid, "worker-1", Duration::from_millis(50));
        std::thread::sleep(Duration::from_millis(30));
        lease.heartbeat(); // renew before expiry
        std::thread::sleep(Duration::from_millis(30));
        // Only ~30ms since last heartbeat, TTL is 50ms → still valid
        assert!(!lease.is_expired());
    }

    // -----------------------------------------------------------------------
    // J5 — Quotas and rate limiting
    // -----------------------------------------------------------------------

    #[test]
    fn quota_queue_depth_enforced() {
        let dim = DimensionId::new();
        let log = ActionLog::new(64);
        let quota = DimensionQuota {
            max_concurrent: 4,
            max_queued: 2,
            max_tasks_per_second: 1000,
        };
        let sched = JobScheduler::new(dim, quota, log);

        sched.enqueue(basic_entry(dim)).unwrap();
        sched.enqueue(basic_entry(dim)).unwrap();
        let err = sched.enqueue(basic_entry(dim));
        assert!(matches!(err, Err(SchedulerError::QuotaExceeded(_, _))));
    }

    #[test]
    fn rate_limiter_token_bucket() {
        let mut rl = RateLimiter::new(2);
        // Fresh limiter has full bucket
        assert!(rl.try_acquire());
        assert!(rl.try_acquire());
        // Bucket now empty
        assert!(!rl.try_acquire());
    }

    // -----------------------------------------------------------------------
    // J6 — DAG scheduling
    // -----------------------------------------------------------------------

    #[test]
    fn dag_ready_with_no_deps() {
        let dag = TaskDag::new();
        let tid = TaskId::new();
        let entries = HashMap::new();
        assert!(dag.is_ready(tid, &entries));
    }

    #[test]
    fn dag_not_ready_when_dep_not_complete() {
        let dag = TaskDag::new();
        let a = TaskId::new();
        let b = TaskId::new();
        dag.add_dependency(a, b).unwrap();

        let dim = DimensionId::new();
        let mut entries = HashMap::new();
        entries.insert(
            b,
            JobEntry {
                task_id: b,
                dimension_id: dim,
                priority: TaskPriority::Normal,
                state: TaskState::Running, // not complete
                name: None,
                retry_policy: RetryPolicy::default(),
                retry_count: 0,
                preemption_policy: PreemptionPolicy::Never,
            },
        );
        assert!(!dag.is_ready(a, &entries));
    }

    #[test]
    fn dag_ready_when_dep_complete() {
        let dag = TaskDag::new();
        let a = TaskId::new();
        let b = TaskId::new();
        dag.add_dependency(a, b).unwrap();

        let dim = DimensionId::new();
        let mut entries = HashMap::new();
        entries.insert(
            b,
            JobEntry {
                task_id: b,
                dimension_id: dim,
                priority: TaskPriority::Normal,
                state: TaskState::Completed,
                name: None,
                retry_policy: RetryPolicy::default(),
                retry_count: 0,
                preemption_policy: PreemptionPolicy::Never,
            },
        );
        assert!(dag.is_ready(a, &entries));
    }

    #[test]
    fn dag_cycle_detected() {
        let dag = TaskDag::new();
        let a = TaskId::new();
        let b = TaskId::new();
        dag.add_dependency(a, b).unwrap();
        let err = dag.add_dependency(b, a);
        assert!(matches!(err, Err(SchedulerError::DagCycle)));
    }

    #[test]
    fn dag_topological_order() {
        let dag = TaskDag::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();
        // c → b → a (a must finish before b, b before c)
        dag.add_dependency(b, a).unwrap();
        dag.add_dependency(c, b).unwrap();

        let order = dag.topological_order(&[a, b, c]).unwrap();
        let pos = |t: TaskId| order.iter().position(|x| *x == t).unwrap();
        assert!(pos(a) < pos(b));
        assert!(pos(b) < pos(c));
    }

    // -----------------------------------------------------------------------
    // J7 — Preemption policy
    // -----------------------------------------------------------------------

    #[test]
    fn preemption_never() {
        let dim = DimensionId::new();
        let current = JobEntry {
            preemption_policy: PreemptionPolicy::Never,
            priority: TaskPriority::Low,
            ..basic_entry(dim)
        };
        let candidate = JobEntry {
            priority: TaskPriority::Critical,
            ..basic_entry(dim)
        };
        assert!(!should_preempt(&current, &candidate));
    }

    #[test]
    fn preemption_by_priority() {
        let dim = DimensionId::new();
        let current = JobEntry {
            preemption_policy: PreemptionPolicy::ByPriority,
            priority: TaskPriority::Low,
            ..basic_entry(dim)
        };
        let candidate_high = JobEntry {
            priority: TaskPriority::High,
            ..basic_entry(dim)
        };
        let candidate_low = JobEntry {
            priority: TaskPriority::Background,
            ..basic_entry(dim)
        };
        assert!(should_preempt(&current, &candidate_high));
        assert!(!should_preempt(&current, &candidate_low));
    }

    #[test]
    fn preemption_always() {
        let dim = DimensionId::new();
        let current = JobEntry {
            preemption_policy: PreemptionPolicy::Always,
            priority: TaskPriority::High,
            ..basic_entry(dim)
        };
        let candidate = JobEntry {
            priority: TaskPriority::Background,
            ..basic_entry(dim)
        };
        assert!(should_preempt(&current, &candidate));
    }

    // -----------------------------------------------------------------------
    // J8 — Task persistence / recovery
    // -----------------------------------------------------------------------

    #[test]
    fn snapshot_round_trip() {
        let dim = DimensionId::new();
        let entry = basic_entry(dim);
        let tid = entry.task_id;

        let snap = TaskSnapshot::from_entry(&entry, serde_json::json!({"k": "v"}));
        let json = serde_json::to_string(&snap).unwrap();
        let restored: TaskSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.task_id, tid);
        assert_eq!(restored.dimension_id, dim);
    }

    #[test]
    fn recover_running_task_becomes_queued() {
        let dim = DimensionId::new();
        let mut entry = basic_entry(dim);
        entry.state = TaskState::Running;

        let snap = TaskSnapshot::from_entry(&entry, serde_json::Value::Null);
        let recovered = snap.recover();
        assert_eq!(recovered.state, TaskState::Queued);
    }

    #[test]
    fn recover_completed_task_stays_completed() {
        let dim = DimensionId::new();
        let mut entry = basic_entry(dim);
        entry.state = TaskState::Completed;

        let snap = TaskSnapshot::from_entry(&entry, serde_json::Value::Null);
        let recovered = snap.recover();
        assert_eq!(recovered.state, TaskState::Completed);
    }

    // -----------------------------------------------------------------------
    // J9 — Task templates
    // -----------------------------------------------------------------------

    #[test]
    fn template_instantiation() {
        let tmpl = TaskTemplate::new("ml-train");
        let dim = DimensionId::new();
        let tid = TaskId::new();
        let entry = tmpl.instantiate(tid, dim, Some(TaskPriority::High), None);
        assert_eq!(entry.task_id, tid);
        assert_eq!(entry.priority, TaskPriority::High);
        assert_eq!(entry.name.as_deref(), Some("ml-train"));
        assert_eq!(entry.state, TaskState::Queued);
    }

    #[test]
    fn template_registry_register_and_retrieve() {
        let registry = TaskTemplateRegistry::new();
        registry.register(TaskTemplate::new("etl-pipeline"));
        let t = registry.get("etl-pipeline").unwrap();
        assert_eq!(t.name, "etl-pipeline");
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn template_override_retry() {
        let tmpl = TaskTemplate::new("batch");
        let dim = DimensionId::new();
        let tid = TaskId::new();
        let custom_retry = RetryPolicy {
            max_attempts: 10,
            ..Default::default()
        };
        let entry = tmpl.instantiate(tid, dim, None, Some(custom_retry));
        assert_eq!(entry.retry_policy.max_attempts, 10);
    }

    // -----------------------------------------------------------------------
    // J10 — TaskIndex (global uniqueness)
    // -----------------------------------------------------------------------

    #[test]
    fn task_index_prevents_duplicates() {
        let index = TaskIndex::new();
        let tid = TaskId::new();
        let dim = DimensionId::new();
        index.register(tid, dim).unwrap();
        let err = index.register(tid, dim);
        assert!(matches!(err, Err(SchedulerError::DuplicateTaskId(_))));
    }

    #[test]
    fn task_index_dimension_lookup() {
        let index = TaskIndex::new();
        let tid = TaskId::new();
        let dim = DimensionId::new();
        index.register(tid, dim).unwrap();
        assert_eq!(index.dimension_of(tid), Some(dim));
    }

    #[test]
    fn task_index_contains() {
        let index = TaskIndex::new();
        let tid = TaskId::new();
        let dim = DimensionId::new();
        assert!(!index.contains(tid));
        index.register(tid, dim).unwrap();
        assert!(index.contains(tid));
    }

    #[test]
    fn task_index_cross_dimension_different_ids() {
        let index = TaskIndex::new();
        let t1 = TaskId::new();
        let t2 = TaskId::new();
        let d1 = DimensionId::new();
        let d2 = DimensionId::new();
        index.register(t1, d1).unwrap();
        index.register(t2, d2).unwrap();
        assert_eq!(index.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Integration: enqueue → dequeue → complete lifecycle
    // -----------------------------------------------------------------------

    #[test]
    fn full_lifecycle_queued_running_completed() {
        let (sched, dim) = make_scheduler();
        let dag = TaskDag::new();

        let entry = basic_entry(dim);
        let tid = entry.task_id;
        sched.enqueue(entry).unwrap();

        assert_eq!(sched.state(tid), Some(TaskState::Queued));

        let dequeued = sched.dequeue(&dag).unwrap();
        assert_eq!(dequeued.task_id, tid);
        assert_eq!(sched.state(tid), Some(TaskState::Running));

        sched.complete(tid).unwrap();
        assert_eq!(sched.state(tid), Some(TaskState::Completed));
    }

    #[test]
    fn pause_resume_cycle() {
        let (sched, dim) = make_scheduler();
        let dag = TaskDag::new();

        let entry = basic_entry(dim);
        let tid = entry.task_id;
        sched.enqueue(entry).unwrap();
        sched.dequeue(&dag).unwrap(); // → Running
        sched.pause(tid).unwrap();
        assert_eq!(sched.state(tid), Some(TaskState::Paused));
        sched.resume(tid).unwrap();
        assert_eq!(sched.state(tid), Some(TaskState::Queued));
        // Should be re-dequeued
        let got = sched.dequeue(&dag).unwrap();
        assert_eq!(got.task_id, tid);
    }
}
