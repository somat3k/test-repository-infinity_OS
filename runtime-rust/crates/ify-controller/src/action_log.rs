//! ActionLog capture for every controller action.
//!
//! Every controller action MUST emit an [`ActionLogEntry`] before returning.
//! The [`ActionLog`] is an in-process, append-only sink.  Downstream adapters
//! (DB, mesh, telemetry) subscribe via [`ActionLog::subscribe`].
//!
//! ## Schema alignment
//!
//! The [`ActionLogEntry`] fields match the envelope defined in
//! `docs/architecture/event-taxonomy.md`:
//!
//! | Spec field       | Rust field           |
//! |------------------|----------------------|
//! | `event_id`       | `event_id`           |
//! | `event_type`     | `event_type`         |
//! | `occurred_at_ms` | `occurred_at_ms`     |
//! | `dimension_id`   | `dimension_id`       |
//! | `task_id`        | `task_id`            |
//! | `actor`          | `actor`              |
//! | `causality_id`   | `causality_id`       |
//! | `correlation_id` | `correlation_id`     |
//! | `payload`        | `payload`            |

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use ify_core::{ArtifactId, DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{debug, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// EventType
// ---------------------------------------------------------------------------

/// Structured event type identifier, serialised as "verb.noun" strings.
///
/// Variants correspond to the taxonomy defined in
/// `docs/architecture/event-taxonomy.md §3`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    // --- Controller events (§3.6) ---
    /// A blockController was registered.
    ControllerRegistered,
    /// Controller linked to a node or editor.
    ControllerLinked,
    /// Controller isolated (sandbox mode).
    ControllerIsolated,
    /// Controller lifecycle ended.
    ControllerDisposed,

    // --- Task events (§3.1) ---
    /// A task was accepted into the queue.
    TaskSubmitted,
    /// Execution began.
    TaskStarted,
    /// Task finished successfully.
    TaskCompleted,
    /// Task terminated with an error.
    TaskFailed,
    /// Task was cancelled.
    TaskCancelled,
    /// Task was re-submitted after failure.
    TaskRetried,

    // --- Node / graph events (§3.4) ---
    /// A canvas node was added.
    NodeCreated,
    /// A node's parameters changed.
    NodeUpdated,
    /// A node was removed from the graph.
    NodeDeleted,
    /// Undo command applied to the node graph.
    NodeUndo,
    /// Redo command applied to the node graph.
    NodeRedo,

    // --- Flow control events ---
    /// A flow control decision was evaluated.
    FlowEvaluated,
    /// Flow advanced to the next step.
    FlowAdvanced,

    // --- Model runtime events ---
    /// Hyperparameters were adjusted based on performance.
    ModelHyperparametersAdjusted,
    /// A model reload was requested based on performance.
    ModelReloadRequested,

    // --- Replica events ---
    /// A kernel replica was provisioned for a model module.
    ReplicaProvisioned,
    /// A kernel replica was released.
    ReplicaReleased,

    // --- Chat events ---
    /// A chat request was received for payload adaptation.
    ChatRequestReceived,
    /// A chat payload was generated for downstream execution.
    ChatPayloadGenerated,

    // --- Artifact events (§3.3) ---
    /// An artifact was committed to storage.
    ArtifactProduced,
    /// An artifact was read by a task or agent.
    ArtifactConsumed,
    /// A node snapshot was captured.
    ArtifactSnapshot,
    /// A diff/patch was applied and stored.
    ArtifactPatched,

    // --- Block registration events ---
    /// A new editor instance was created.
    EditorCreated,
    /// An interpreter was attached to an editor.
    InterpreterAttached,
    /// A block was bound to the executor runtime.
    RuntimeBound,

    // --- Orchestrator events ---
    /// A task was submitted to the orchestrator.
    OrchestratorSubmit,
    /// A task was cancelled via the orchestrator.
    OrchestratorCancel,
    /// Orchestrator replayed stored events for a task.
    OrchestratorReplay,
    /// Progress event emitted by the orchestrator.
    OrchestratorProgress,

    // --- Graph connectivity events (Epic F) ---
    /// A directed link was added between two node ports.
    LinkCreated,
    /// A directed link was removed from the graph.
    LinkRemoved,
    /// A node group was created.
    GroupCreated,
    /// A node group was removed.
    GroupRemoved,
    /// A subgraph / macro was registered.
    SubgraphCreated,
    /// The graph passed or failed validation.
    GraphValidated,
    /// A diff/patch was applied to the flow graph.
    GraphPatched,

    // --- Node execution contract events (Epic F) ---
    /// A node execution contract transitioned to Running.
    NodeExecutionStarted,
    /// A node execution contract emitted a progress update.
    NodeExecutionProgress,
    /// A node execution contract reached Complete.
    NodeExecutionCompleted,
    /// A node execution contract reached Failed.
    NodeExecutionFailed,
    /// A node execution contract was Cancelled.
    NodeExecutionCancelled,

    // --- Node relation events (Epic F / node communication) ---
    /// A semantic relation was created between two nodes.
    NodeRelationCreated,
    /// A semantic relation between two nodes was removed.
    NodeRelationRemoved,

    // --- Node communication / messaging events ---
    /// A message was sent from one node to another.
    NodeMessageSent,
    /// A message was broadcast from a node to all listeners.
    NodeMessageBroadcast,

    // --- Epic J: Job Scheduling events ---
    /// A task lease was renewed by a worker heartbeat.
    TaskLeaseRenewed,
    /// A task was paused (e.g., preemption).
    TaskPaused,
    /// A paused task was resumed.
    TaskResumed,

    // --- Epic N: Node Instance Grouping events ---
    /// An instance template was created.
    TemplateCreated,
    /// An instance template was cloned.
    TemplateCloned,
    /// An instance template was forked.
    TemplateForked,
    /// A node instance was expanded from a template.
    InstanceCreated,
}

impl EventType {
    /// Return the canonical "verb.noun" string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ControllerRegistered => "controller.registered",
            Self::ControllerLinked => "controller.linked",
            Self::ControllerIsolated => "controller.isolated",
            Self::ControllerDisposed => "controller.disposed",
            Self::TaskSubmitted => "task.submitted",
            Self::TaskStarted => "task.started",
            Self::TaskCompleted => "task.completed",
            Self::TaskFailed => "task.failed",
            Self::TaskCancelled => "task.cancelled",
            Self::TaskRetried => "task.retried",
            Self::NodeCreated => "node.created",
            Self::NodeUpdated => "node.updated",
            Self::NodeDeleted => "node.deleted",
            Self::NodeUndo => "node.undo",
            Self::NodeRedo => "node.redo",
            Self::FlowEvaluated => "flow.evaluated",
            Self::FlowAdvanced => "flow.advanced",
            Self::ModelHyperparametersAdjusted => "model.hyperparameters_adjusted",
            Self::ModelReloadRequested => "model.reload_requested",
            Self::ReplicaProvisioned => "replica.provisioned",
            Self::ReplicaReleased => "replica.released",
            Self::ChatRequestReceived => "chat.request_received",
            Self::ChatPayloadGenerated => "chat.payload_generated",
            Self::ArtifactProduced => "artifact.produced",
            Self::ArtifactConsumed => "artifact.consumed",
            Self::ArtifactSnapshot => "artifact.snapshot",
            Self::ArtifactPatched => "artifact.patched",
            Self::EditorCreated => "editor.created",
            Self::InterpreterAttached => "interpreter.attached",
            Self::RuntimeBound => "runtime.bound",
            Self::OrchestratorSubmit => "orchestrator.submit",
            Self::OrchestratorCancel => "orchestrator.cancel",
            Self::OrchestratorReplay => "orchestrator.replay",
            Self::OrchestratorProgress => "orchestrator.progress",
            Self::LinkCreated => "link.created",
            Self::LinkRemoved => "link.removed",
            Self::GroupCreated => "group.created",
            Self::GroupRemoved => "group.removed",
            Self::SubgraphCreated => "subgraph.created",
            Self::GraphValidated => "graph.validated",
            Self::GraphPatched => "graph.patched",
            Self::NodeExecutionStarted => "node_execution.started",
            Self::NodeExecutionProgress => "node_execution.progress",
            Self::NodeExecutionCompleted => "node_execution.completed",
            Self::NodeExecutionFailed => "node_execution.failed",
            Self::NodeExecutionCancelled => "node_execution.cancelled",
            Self::NodeRelationCreated => "node_relation.created",
            Self::NodeRelationRemoved => "node_relation.removed",
            Self::NodeMessageSent => "node_message.sent",
            Self::NodeMessageBroadcast => "node_message.broadcast",
            Self::TaskLeaseRenewed => "task.lease_renewed",
            Self::TaskPaused => "task.paused",
            Self::TaskResumed => "task.resumed",
            Self::TemplateCreated => "template.created",
            Self::TemplateCloned => "template.cloned",
            Self::TemplateForked => "template.forked",
            Self::InstanceCreated => "instance.created",
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Actor
// ---------------------------------------------------------------------------

/// The kind of actor that triggered a controller event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Actor {
    /// A human user identified by an opaque string.
    User(String),
    /// An automated agent identified by an opaque string.
    Agent(String),
    /// A system/background process.
    System,
    /// The kernel layer.
    Kernel,
}

// ---------------------------------------------------------------------------
// ActionLogEntry
// ---------------------------------------------------------------------------

/// A single immutable ActionLog entry.
///
/// All fields mirror the envelope from `docs/architecture/event-taxonomy.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLogEntry {
    /// Unique ID for this log entry (UUID v7, time-ordered).
    pub event_id: ArtifactId,
    /// Structured event type identifier ("verb.noun").
    pub event_type: EventType,
    /// Unix epoch milliseconds when this event occurred.
    pub occurred_at_ms: u64,
    /// Dimension scope for this event (`None` for kernel-level events).
    pub dimension_id: Option<DimensionId>,
    /// Task context for this event (`None` if not task-scoped).
    pub task_id: Option<TaskId>,
    /// Actor that triggered the event.
    pub actor: Actor,
    /// `event_id` of the event that directly caused this one.
    pub causality_id: Option<Uuid>,
    /// Groups all events belonging to the same user-facing operation.
    pub correlation_id: Option<String>,
    /// Verb-specific payload fields.
    pub payload: serde_json::Value,
}

impl ActionLogEntry {
    /// Construct a new entry with a fresh `event_id` and the current timestamp.
    pub fn new(
        event_type: EventType,
        actor: Actor,
        dimension_id: Option<DimensionId>,
        task_id: Option<TaskId>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            event_id: ArtifactId::new(),
            event_type,
            occurred_at_ms: now_ms(),
            dimension_id,
            task_id,
            actor,
            causality_id: None,
            correlation_id: None,
            payload,
        }
    }

    /// Set the causality ID (chained from a previous event).
    #[must_use]
    pub fn with_causality(mut self, causality_id: Uuid) -> Self {
        self.causality_id = Some(causality_id);
        self
    }

    /// Set the correlation ID (groups a user-facing operation).
    #[must_use]
    pub fn with_correlation(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }
}

// ---------------------------------------------------------------------------
// ActionLog
// ---------------------------------------------------------------------------

/// Thread-safe, append-only ActionLog.
///
/// Stores entries in memory and broadcasts each new entry to all active
/// subscribers.  Production deployments should add a persistence adapter
/// by subscribing and flushing to durable storage.
pub struct ActionLog {
    entries: Mutex<Vec<ActionLogEntry>>,
    tx: broadcast::Sender<ActionLogEntry>,
}

impl std::fmt::Debug for ActionLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self
            .entries
            .lock()
            .map(|g| g.len())
            .unwrap_or(0);
        write!(f, "ActionLog {{ entries: {} }}", len)
    }
}

impl ActionLog {
    /// Create a new `ActionLog`.
    ///
    /// `capacity` controls the broadcast channel's ring-buffer size.  Slow
    /// subscribers that fall behind will miss entries; they will receive a
    /// [`broadcast::error::RecvError::Lagged`] error.
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Arc::new(Self {
            entries: Mutex::new(Vec::new()),
            tx,
        })
    }

    /// Append an entry to the log and broadcast it to all subscribers.
    pub fn append(&self, entry: ActionLogEntry) {
        debug!(
            event_type = %entry.event_type,
            event_id = %entry.event_id,
            "action_log append"
        );
        {
            let mut guard = self.entries.lock().expect("action_log lock poisoned");
            guard.push(entry.clone());
        }
        if let Err(e) = self.tx.send(entry) {
            // No subscribers is normal; only warn if it looks unexpected.
            if self.tx.receiver_count() > 0 {
                warn!("action_log broadcast failed: {e}");
            }
        }
    }

    /// Subscribe to new entries.  The receiver will see every entry appended
    /// *after* this call; use [`entries_for_dimension`] to replay history.
    pub fn subscribe(&self) -> broadcast::Receiver<ActionLogEntry> {
        self.tx.subscribe()
    }

    /// Return all entries scoped to the given dimension.
    pub fn entries_for_dimension(&self, dim: DimensionId) -> Vec<ActionLogEntry> {
        self.entries
            .lock()
            .expect("action_log lock poisoned")
            .iter()
            .filter(|e| e.dimension_id == Some(dim))
            .cloned()
            .collect()
    }

    /// Return all entries associated with a specific task.
    pub fn entries_for_task(&self, task_id: TaskId) -> Vec<ActionLogEntry> {
        self.entries
            .lock()
            .expect("action_log lock poisoned")
            .iter()
            .filter(|e| e.task_id == Some(task_id))
            .cloned()
            .collect()
    }

    /// Return a snapshot of all entries (newest last).
    pub fn all_entries(&self) -> Vec<ActionLogEntry> {
        self.entries
            .lock()
            .expect("action_log lock poisoned")
            .clone()
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("action_log lock poisoned")
            .len()
    }

    /// Returns `true` when no entries have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Current Unix epoch in milliseconds.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::{DimensionId, TaskId};

    fn make_entry(dim: DimensionId, task: TaskId) -> ActionLogEntry {
        ActionLogEntry::new(
            EventType::ControllerRegistered,
            Actor::System,
            Some(dim),
            Some(task),
            serde_json::json!({"controller_kind": "block", "dimension_id": dim.to_string()}),
        )
    }

    #[test]
    fn append_and_query() {
        let log = ActionLog::new(16);
        let dim = DimensionId::new();
        let task = TaskId::new();

        log.append(make_entry(dim, task));
        log.append(make_entry(dim, task));
        log.append(make_entry(DimensionId::new(), TaskId::new()));

        assert_eq!(log.len(), 3);
        assert_eq!(log.entries_for_dimension(dim).len(), 2);
        assert_eq!(log.entries_for_task(task).len(), 2);
    }

    #[test]
    fn entry_has_unique_event_ids() {
        let log = ActionLog::new(8);
        let dim = DimensionId::new();
        let task = TaskId::new();

        log.append(make_entry(dim, task));
        log.append(make_entry(dim, task));

        let entries = log.all_entries();
        assert_ne!(entries[0].event_id, entries[1].event_id);
    }

    #[test]
    fn event_type_display_matches_spec() {
        assert_eq!(EventType::ControllerRegistered.as_str(), "controller.registered");
        assert_eq!(EventType::TaskSubmitted.as_str(), "task.submitted");
        assert_eq!(EventType::ArtifactProduced.as_str(), "artifact.produced");
        assert_eq!(EventType::NodeCreated.as_str(), "node.created");
    }

    #[test]
    fn causality_and_correlation_chain() {
        let log = ActionLog::new(8);
        let dim = DimensionId::new();
        let task = TaskId::new();
        let cause_id = Uuid::new_v4();

        let entry = ActionLogEntry::new(
            EventType::TaskStarted,
            Actor::Agent("agent-1".into()),
            Some(dim),
            Some(task),
            serde_json::json!({}),
        )
        .with_causality(cause_id)
        .with_correlation("req-abc");

        assert_eq!(entry.causality_id, Some(cause_id));
        assert_eq!(entry.correlation_id.as_deref(), Some("req-abc"));

        log.append(entry);
        assert_eq!(log.len(), 1);
    }

    #[tokio::test]
    async fn subscribe_receives_new_entries() {
        let log = ActionLog::new(16);
        let mut rx = log.subscribe();

        let dim = DimensionId::new();
        let task = TaskId::new();
        log.append(make_entry(dim, task));

        let received = rx.recv().await.expect("should receive entry");
        assert_eq!(received.event_type, EventType::ControllerRegistered);
    }
}
