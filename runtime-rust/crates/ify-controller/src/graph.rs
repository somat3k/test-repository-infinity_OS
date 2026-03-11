//! Flow Graph and Node Connectivity — Epic F
//!
//! Provides the complete flow graph data model for infinityOS:
//!
//! * **Typed ports** — every node exposes named, typed, directional ports.
//! * **Links** — directed edges that connect an output port to an input port.
//! * **Groups** — named collections of nodes for organisational grouping.
//! * **Subgraphs / macros** — versioned, parameterisable sub-graphs that can
//!   be embedded inside a parent graph via a [`GraphNode`] reference.
//! * **Deterministic serialisation** — all maps use [`BTreeMap`] so the JSON
//!   representation has stable key ordering. A `schema_version` field guards
//!   against deserialising an incompatible wire format.
//! * **Cycle detection** — depth-first search with a recursion-stack colour.
//! * **Execution-order planning** — Kahn's topological sort yields a
//!   deterministic execution sequence.
//! * **Node execution contracts** — explicit state machine
//!   (`Idle → Running → Progress* → Complete | Failed | Cancelled`).
//! * **Graph diff / patch** — a typed list of [`GraphPatchOp`]s can be
//!   applied atomically (all-or-nothing) to a [`FlowGraphSchema`].
//! * **Graph validation** — type compatibility, required-port connectivity,
//!   forbidden-edge policies.
//! * **Node provenance** — every node carries who/what/when/why via
//!   [`NodeProvenance`], and all mutating operations emit an
//!   [`ActionLogEntry`].

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Schema version
// ---------------------------------------------------------------------------

/// Wire-format schema version for [`FlowGraphSchema`].
///
/// Bump this whenever the serialised layout changes in a backwards-incompatible
/// way and add a migration path.
pub const GRAPH_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return current Unix epoch in milliseconds.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by flow graph operations.
#[derive(Debug, Error)]
pub enum FlowGraphError {
    /// A referenced node was not found in the graph.
    #[error("node {0} not found in graph")]
    NodeNotFound(Uuid),

    /// A referenced link was not found in the graph.
    #[error("link {0} not found in graph")]
    LinkNotFound(Uuid),

    /// A referenced port was not found on the given node.
    #[error("port {port_id} not found on node {node_id}")]
    PortNotFound {
        /// Owning node.
        node_id: Uuid,
        /// Missing port.
        port_id: Uuid,
    },

    /// A referenced group was not found.
    #[error("group {0} not found in graph")]
    GroupNotFound(Uuid),

    /// A referenced subgraph was not found.
    #[error("subgraph {0} not found in graph")]
    SubgraphNotFound(Uuid),

    /// A referenced node relation was not found.
    #[error("relation {0} not found in graph")]
    RelationNotFound(Uuid),

    /// The graph contains a cycle; the payload lists the cycle node IDs.
    #[error("graph contains a cycle through nodes: {0:?}")]
    CycleDetected(Vec<Uuid>),

    /// Port types are incompatible across a link.
    #[error(
        "type mismatch on link {link_id}: source port has type {source_type:?}, \
         target port expects {target_type:?}"
    )]
    TypeMismatch {
        /// Offending link.
        link_id: Uuid,
        /// Type on the output side.
        source_type: PortDataType,
        /// Type on the input side.
        target_type: PortDataType,
    },

    /// A required input port has no incoming link.
    #[error("node {node_id} has required input port '{port_name}' with no incoming link")]
    UnconnectedRequiredPort {
        /// Owning node.
        node_id: Uuid,
        /// Required port name.
        port_name: String,
    },

    /// An edge is forbidden by graph policy.
    #[error("forbidden edge from node {from_node} to {to_node}: {reason}")]
    ForbiddenEdge {
        /// Source node.
        from_node: Uuid,
        /// Target node.
        to_node: Uuid,
        /// Policy reason.
        reason: String,
    },

    /// The graph patch contained an invalid or inapplicable operation.
    #[error("invalid patch operation: {0}")]
    InvalidPatch(String),

    /// The graph schema version does not match.
    #[error("schema version mismatch: expected {expected}, found {found}")]
    SchemaMismatch {
        /// Version this code handles.
        expected: u32,
        /// Version in the data.
        found: u32,
    },

    /// A node execution state transition is not permitted.
    #[error("invalid execution state transition for node {node_id}: {from:?} → {to:?}")]
    InvalidStateTransition {
        /// Node whose contract was violated.
        node_id: Uuid,
        /// Current state label.
        from: &'static str,
        /// Requested target state label.
        to: &'static str,
    },

    /// A port name is used more than once on the same node.
    #[error("duplicate port name '{name}' on node {node_id}")]
    DuplicatePortName {
        /// Owning node.
        node_id: Uuid,
        /// Duplicated name.
        name: String,
    },

    /// The provided dimension ID does not match this graph's dimension.
    #[error("dimension mismatch: graph owns {expected}, caller provided {got}")]
    DimensionMismatch {
        /// Dimension this graph owns.
        expected: DimensionId,
        /// Dimension supplied by the caller.
        got: DimensionId,
    },
}

// ---------------------------------------------------------------------------
// Port model
// ---------------------------------------------------------------------------

/// Whether data flows into or out of a node through a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PortDirection {
    /// Data enters the node through this port.
    In,
    /// Data leaves the node through this port.
    Out,
}

/// Primitive data type carried by a port.
///
/// `Any` bypasses type checking and is compatible with all other types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDataType {
    /// Matches any type (bypass checking).
    Any,
    /// 64-bit IEEE-754 floating-point number.
    Number,
    /// UTF-8 string.
    String,
    /// Boolean flag.
    Bool,
    /// Arbitrary JSON object.
    Json,
    /// Ordered JSON array.
    Array,
}

impl PortDataType {
    /// Returns `true` if `self` and `other` can be connected.
    ///
    /// `Any` is compatible with every type.  Otherwise both types must match.
    pub fn is_compatible_with(self, other: PortDataType) -> bool {
        self == PortDataType::Any || other == PortDataType::Any || self == other
    }
}

/// Typed port definition attached to a [`GraphNode`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDef {
    /// Unique port identifier (stable across serialise/deserialise cycles).
    pub id: Uuid,
    /// Human-readable name; must be unique within the owning node.
    pub name: String,
    /// Data flow direction.
    pub direction: PortDirection,
    /// Expected data type.
    pub data_type: PortDataType,
    /// If `true`, an incoming link (for `In` ports) is required before the
    /// node can be executed.
    pub required: bool,
    /// Optional human-readable description.
    pub description: String,
}

impl PortDef {
    /// Create a new non-required port with an empty description.
    pub fn new(
        name: impl Into<String>,
        direction: PortDirection,
        data_type: PortDataType,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            direction,
            data_type,
            required: false,
            description: String::new(),
        }
    }

    /// Mark this port as required and return `self` (builder pattern).
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Attach a description and return `self` (builder pattern).
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
}

// ---------------------------------------------------------------------------
// Link
// ---------------------------------------------------------------------------

/// Directed edge connecting an output port on one node to an input port on
/// another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// Unique link identifier.
    pub id: Uuid,
    /// ID of the node that emits data.
    pub source_node: Uuid,
    /// Output port on the source node.
    pub source_port: Uuid,
    /// ID of the node that receives data.
    pub target_node: Uuid,
    /// Input port on the target node.
    pub target_port: Uuid,
    /// Optional human-readable label.
    pub label: Option<String>,
}

impl Link {
    /// Construct a new link between two named ports.
    pub fn new(
        source_node: Uuid,
        source_port: Uuid,
        target_node: Uuid,
        target_port: Uuid,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            source_node,
            source_port,
            target_node,
            target_port,
            label: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Group
// ---------------------------------------------------------------------------

/// Named collection of node IDs for organisational grouping on the canvas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Unique group identifier.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Nodes belonging to this group (stored as a sorted set for stable
    /// serialisation).
    pub node_ids: BTreeSet<Uuid>,
}

impl Group {
    /// Create an empty group.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            name: name.into(),
            node_ids: BTreeSet::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Node relations
// ---------------------------------------------------------------------------

/// The semantic kind of a named relationship between two nodes.
///
/// Relations are distinct from data-flow [`Link`]s — they model higher-level
/// intent (e.g. "this node *depends on* that node") without implying direct
/// port wiring.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// The `from_node` cannot run until `to_node` has completed successfully.
    DependsOn,
    /// Completion of `from_node` triggers execution of `to_node`.
    Triggers,
    /// `to_node` monitors / samples outputs from `from_node`.
    ObservedBy,
    /// `from_node` is the canonical data source consumed by `to_node`.
    ProvidesDataTo,
    /// Application-defined relationship with a free-form label.
    Custom(String),
}

impl RelationKind {
    /// Return a stable, human-readable string for the relation kind.
    pub fn as_str(&self) -> &str {
        match self {
            Self::DependsOn => "depends_on",
            Self::Triggers => "triggers",
            Self::ObservedBy => "observed_by",
            Self::ProvidesDataTo => "provides_data_to",
            Self::Custom(s) => s.as_str(),
        }
    }
}

/// A named, typed semantic relationship between two nodes.
///
/// Relations complement port-based [`Link`]s by expressing *intent*
/// independently of data flow.  They are stored in [`FlowGraphSchema::relations`]
/// and emitted to the [`ActionLog`] on creation and removal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRelation {
    /// Unique relation identifier.
    pub id: Uuid,
    /// Source node of the relationship.
    pub from_node: Uuid,
    /// Target node of the relationship.
    pub to_node: Uuid,
    /// Semantic kind of the relationship.
    pub kind: RelationKind,
    /// Optional human-readable label.
    pub label: Option<String>,
    /// Arbitrary metadata attached to the relation.
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl NodeRelation {
    /// Construct a new relation between two nodes.
    pub fn new(from_node: Uuid, to_node: Uuid, kind: RelationKind) -> Self {
        Self {
            id: Uuid::now_v7(),
            from_node,
            to_node,
            kind,
            label: None,
            metadata: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Node provenance
// ---------------------------------------------------------------------------

/// Provenance record attached to every graph node.
///
/// Answers the audit questions: *who* created it, *what* task caused it,
/// *when* was it created, and *why*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeProvenance {
    /// Actor (user login or agent ID) that created the node.
    pub created_by: String,
    /// Unix epoch milliseconds at creation time.
    pub created_at_ms: u64,
    /// Actor that last modified the node, if any.
    pub modified_by: Option<String>,
    /// Unix epoch milliseconds at last modification, if any.
    pub modified_at_ms: Option<u64>,
    /// Human-readable reason for creating the node.
    pub reason: Option<String>,
    /// Task that caused the creation, if applicable.
    pub task_id: Option<String>,
}

impl NodeProvenance {
    /// Create a provenance record attributed to the given actor.
    pub fn for_actor(actor: impl Into<String>) -> Self {
        Self {
            created_by: actor.into(),
            created_at_ms: now_ms(),
            modified_by: None,
            modified_at_ms: None,
            reason: None,
            task_id: None,
        }
    }

    /// Record a modification by the given actor at the current time.
    pub fn touch(&mut self, actor: impl Into<String>) {
        self.modified_by = Some(actor.into());
        self.modified_at_ms = Some(now_ms());
    }
}

impl Default for NodeProvenance {
    fn default() -> Self {
        Self::for_actor("system")
    }
}

// ---------------------------------------------------------------------------
// GraphNode
// ---------------------------------------------------------------------------

/// A single node in the flow graph with typed ports and parameter storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Unique node identifier.
    pub id: Uuid,
    /// Node kind / type identifier (e.g. `"http.request"`, `"ml.predict"`).
    pub kind: String,
    /// Human-readable canvas label.
    pub label: String,
    /// Typed port definitions keyed by port ID.
    ///
    /// `BTreeMap` ensures stable JSON key ordering.
    pub ports: BTreeMap<Uuid, PortDef>,
    /// Configuration parameters; `BTreeMap` for stable serialisation.
    pub parameters: BTreeMap<String, serde_json::Value>,
    /// 2-D canvas position `(x, y)`.
    pub position: (f64, f64),
    /// If set, this node is a reference to a [`Subgraph`] within the graph.
    pub subgraph_ref: Option<Uuid>,
    /// Provenance: who/what/when/why.
    pub provenance: NodeProvenance,
}

impl GraphNode {
    /// Create a new node with the given kind and label.
    pub fn new(kind: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            kind: kind.into(),
            label: label.into(),
            ports: BTreeMap::new(),
            parameters: BTreeMap::new(),
            position: (0.0, 0.0),
            subgraph_ref: None,
            provenance: NodeProvenance::default(),
        }
    }

    /// Add a port definition and return its ID.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::DuplicatePortName`] if a port with the same
    /// name already exists on this node.
    pub fn add_port(&mut self, port: PortDef) -> Result<Uuid, FlowGraphError> {
        let duplicate = self.ports.values().any(|p| p.name == port.name);
        if duplicate {
            return Err(FlowGraphError::DuplicatePortName {
                node_id: self.id,
                name: port.name.clone(),
            });
        }
        let id = port.id;
        self.ports.insert(id, port);
        Ok(id)
    }

    /// Iterate over all input ports.
    pub fn inputs(&self) -> impl Iterator<Item = &PortDef> {
        self.ports.values().filter(|p| p.direction == PortDirection::In)
    }

    /// Iterate over all output ports.
    pub fn outputs(&self) -> impl Iterator<Item = &PortDef> {
        self.ports.values().filter(|p| p.direction == PortDirection::Out)
    }

    /// Return `true` if this node is a subgraph reference.
    pub fn is_subgraph_ref(&self) -> bool {
        self.subgraph_ref.is_some()
    }
}

// ---------------------------------------------------------------------------
// Node execution contracts
// ---------------------------------------------------------------------------

/// Execution state of a single graph node.
///
/// Transitions:
/// ```text
/// Idle → Running → Progress* → Complete
///                            → Failed
///                            → Cancelled
/// Running → Cancelled
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NodeExecutionState {
    /// Node has not started execution.
    Idle,
    /// Node execution has begun.
    Running {
        /// Unix epoch milliseconds when execution started.
        started_at_ms: u64,
    },
    /// Intermediate progress update.
    Progress {
        /// Completion percentage `[0.0, 100.0]`.
        percent: f64,
        /// Human-readable progress message.
        message: String,
    },
    /// Node finished successfully.
    Complete {
        /// Unix epoch milliseconds when execution finished.
        finished_at_ms: u64,
        /// Structured output produced by the node.
        output: serde_json::Value,
    },
    /// Node terminated with an error.
    Failed {
        /// Human-readable error description.
        error: String,
        /// Unix epoch milliseconds when failure occurred.
        failed_at_ms: u64,
    },
    /// Execution was cancelled before completion.
    Cancelled {
        /// Unix epoch milliseconds when cancellation was recorded.
        cancelled_at_ms: u64,
    },
}

impl NodeExecutionState {
    /// Return a short label used for error messages.
    fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Running { .. } => "Running",
            Self::Progress { .. } => "Progress",
            Self::Complete { .. } => "Complete",
            Self::Failed { .. } => "Failed",
            Self::Cancelled { .. } => "Cancelled",
        }
    }

    /// Returns `true` if this is a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete { .. } | Self::Failed { .. } | Self::Cancelled { .. })
    }
}

/// Node execution contract — manages the lifecycle of a single node run.
///
/// All state transitions emit an [`ActionLogEntry`] so the audit trail is
/// complete.
pub struct NodeExecutionContract {
    /// ID of the node being executed.
    pub node_id: Uuid,
    /// Dimension scope.
    pub dimension_id: DimensionId,
    /// Task scope.
    pub task_id: TaskId,
    state: NodeExecutionState,
    action_log: Arc<ActionLog>,
}

impl NodeExecutionContract {
    /// Create a new contract in the [`NodeExecutionState::Idle`] state.
    pub fn new(
        node_id: Uuid,
        dimension_id: DimensionId,
        task_id: TaskId,
        action_log: Arc<ActionLog>,
    ) -> Self {
        Self {
            node_id,
            dimension_id,
            task_id,
            state: NodeExecutionState::Idle,
            action_log,
        }
    }

    /// Current execution state.
    pub fn state(&self) -> &NodeExecutionState {
        &self.state
    }

    /// Transition from `Idle` → `Running`.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::InvalidStateTransition`] if the current
    /// state is not `Idle`.
    pub fn start(&mut self) -> Result<(), FlowGraphError> {
        if !matches!(self.state, NodeExecutionState::Idle) {
            return Err(FlowGraphError::InvalidStateTransition {
                node_id: self.node_id,
                from: self.state.label(),
                to: "Running",
            });
        }
        self.state = NodeExecutionState::Running { started_at_ms: now_ms() };
        self.action_log.append(
            ActionLogEntry::new(
                EventType::NodeExecutionStarted,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "node_id": self.node_id }),
            )
            .with_correlation(self.node_id.to_string()),
        );
        Ok(())
    }

    /// Emit a `Progress` event from `Running` or `Progress`.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::InvalidStateTransition`] if the node is not
    /// in `Running` or `Progress` state.
    pub fn progress(&mut self, percent: f64, message: impl Into<String>) -> Result<(), FlowGraphError> {
        if !matches!(self.state, NodeExecutionState::Running { .. } | NodeExecutionState::Progress { .. }) {
            return Err(FlowGraphError::InvalidStateTransition {
                node_id: self.node_id,
                from: self.state.label(),
                to: "Progress",
            });
        }
        let message = message.into();
        self.state = NodeExecutionState::Progress { percent, message: message.clone() };
        self.action_log.append(
            ActionLogEntry::new(
                EventType::NodeExecutionProgress,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "node_id": self.node_id, "percent": percent, "message": message }),
            )
            .with_correlation(self.node_id.to_string()),
        );
        Ok(())
    }

    /// Transition to `Complete` from `Running` or `Progress`.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::InvalidStateTransition`] if the node is not
    /// in `Running` or `Progress` state.
    pub fn complete(&mut self, output: serde_json::Value) -> Result<(), FlowGraphError> {
        if !matches!(self.state, NodeExecutionState::Running { .. } | NodeExecutionState::Progress { .. }) {
            return Err(FlowGraphError::InvalidStateTransition {
                node_id: self.node_id,
                from: self.state.label(),
                to: "Complete",
            });
        }
        let finished_at_ms = now_ms();
        self.state = NodeExecutionState::Complete { finished_at_ms, output: output.clone() };
        self.action_log.append(
            ActionLogEntry::new(
                EventType::NodeExecutionCompleted,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "node_id": self.node_id, "output": output }),
            )
            .with_correlation(self.node_id.to_string()),
        );
        Ok(())
    }

    /// Transition to `Failed` from `Running` or `Progress`.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::InvalidStateTransition`] if the node is not
    /// in `Running` or `Progress` state.
    pub fn fail(&mut self, error: impl Into<String>) -> Result<(), FlowGraphError> {
        if !matches!(self.state, NodeExecutionState::Running { .. } | NodeExecutionState::Progress { .. }) {
            return Err(FlowGraphError::InvalidStateTransition {
                node_id: self.node_id,
                from: self.state.label(),
                to: "Failed",
            });
        }
        let error = error.into();
        let failed_at_ms = now_ms();
        self.state = NodeExecutionState::Failed { error: error.clone(), failed_at_ms };
        self.action_log.append(
            ActionLogEntry::new(
                EventType::NodeExecutionFailed,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "node_id": self.node_id, "error": error }),
            )
            .with_correlation(self.node_id.to_string()),
        );
        Ok(())
    }

    /// Transition to `Cancelled` from `Running` or `Progress`.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::InvalidStateTransition`] if the node is not
    /// in `Running` or `Progress` state.
    pub fn cancel(&mut self) -> Result<(), FlowGraphError> {
        if !matches!(self.state, NodeExecutionState::Running { .. } | NodeExecutionState::Progress { .. }) {
            return Err(FlowGraphError::InvalidStateTransition {
                node_id: self.node_id,
                from: self.state.label(),
                to: "Cancelled",
            });
        }
        let cancelled_at_ms = now_ms();
        self.state = NodeExecutionState::Cancelled { cancelled_at_ms };
        self.action_log.append(
            ActionLogEntry::new(
                EventType::NodeExecutionCancelled,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "node_id": self.node_id }),
            )
            .with_correlation(self.node_id.to_string()),
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Subgraph / macro
// ---------------------------------------------------------------------------

/// A versioned, parameterisable sub-graph that can be embedded in a parent
/// graph via a [`GraphNode`] with `subgraph_ref` set.
///
/// The `exposed_inputs` and `exposed_outputs` define the interface boundary
/// visible to the parent graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subgraph {
    /// Unique subgraph identifier.
    pub id: Uuid,
    /// Monotonically increasing version number; bump on incompatible change.
    pub version: u32,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Input ports exposed to the parent graph (sorted for stable output).
    pub exposed_inputs: BTreeMap<Uuid, PortDef>,
    /// Output ports exposed to the parent graph (sorted for stable output).
    pub exposed_outputs: BTreeMap<Uuid, PortDef>,
    /// The inner graph schema of this subgraph.
    pub inner: FlowGraphSchema,
}

impl Subgraph {
    /// Create a new subgraph wrapping the given inner schema.
    pub fn new(name: impl Into<String>, inner: FlowGraphSchema) -> Self {
        Self {
            id: Uuid::now_v7(),
            version: 1,
            name: name.into(),
            description: String::new(),
            exposed_inputs: BTreeMap::new(),
            exposed_outputs: BTreeMap::new(),
            inner,
        }
    }
}

// ---------------------------------------------------------------------------
// FlowGraphSchema — deterministic, versioned wire format
// ---------------------------------------------------------------------------

/// Deterministically serialisable, versioned wire format for a flow graph.
///
/// All collections are [`BTreeMap`] or [`BTreeSet`] to guarantee stable key
/// ordering in the JSON output, enabling content-addressed storage and
/// determinism tests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowGraphSchema {
    /// Schema version — validated on deserialisation.
    pub schema_version: u32,
    /// Unique identifier for this graph.
    pub graph_id: Uuid,
    /// Dimension this graph belongs to.
    pub dimension_id: DimensionId,
    /// Nodes keyed by node ID.
    pub nodes: BTreeMap<Uuid, GraphNode>,
    /// Links keyed by link ID.
    pub links: BTreeMap<Uuid, Link>,
    /// Groups keyed by group ID.
    pub groups: BTreeMap<Uuid, Group>,
    /// Subgraphs / macros keyed by subgraph ID.
    pub subgraphs: BTreeMap<Uuid, Subgraph>,
    /// Semantic node relations keyed by relation ID.
    ///
    /// Deserialises to an empty map when absent (backwards-compatible).
    #[serde(default)]
    pub relations: BTreeMap<Uuid, NodeRelation>,
}

impl FlowGraphSchema {
    /// Create an empty schema for the given dimension.
    pub fn new(dimension_id: DimensionId) -> Self {
        Self {
            schema_version: GRAPH_SCHEMA_VERSION,
            graph_id: Uuid::now_v7(),
            dimension_id,
            nodes: BTreeMap::new(),
            links: BTreeMap::new(),
            groups: BTreeMap::new(),
            subgraphs: BTreeMap::new(),
            relations: BTreeMap::new(),
        }
    }

    /// Serialise to a canonical JSON string (stable key ordering guaranteed).
    ///
    /// # Errors
    ///
    /// Propagates [`serde_json::Error`] on serialisation failure.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialise from a JSON string and validate the schema version.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::SchemaMismatch`] when the version does not
    /// match [`GRAPH_SCHEMA_VERSION`].  Otherwise propagates `serde_json`
    /// errors as [`FlowGraphError::InvalidPatch`].
    pub fn from_json(json: &str) -> Result<Self, FlowGraphError> {
        let schema: Self =
            serde_json::from_str(json).map_err(|e| FlowGraphError::InvalidPatch(e.to_string()))?;
        if schema.schema_version != GRAPH_SCHEMA_VERSION {
            return Err(FlowGraphError::SchemaMismatch {
                expected: GRAPH_SCHEMA_VERSION,
                found: schema.schema_version,
            });
        }
        Ok(schema)
    }
}

// ---------------------------------------------------------------------------
// Graph diff / patch
// ---------------------------------------------------------------------------

/// Atomic patch operation that can be applied to a [`FlowGraphSchema`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GraphPatchOp {
    /// Insert a new node; fails if the ID already exists.
    AddNode(GraphNode),
    /// Remove a node and all attached links; fails if absent.
    RemoveNode {
        /// Node to remove.
        id: Uuid,
    },
    /// Replace the label of an existing node.
    UpdateNodeLabel {
        /// Target node.
        id: Uuid,
        /// New label string.
        new_label: String,
    },
    /// Merge parameters into an existing node (shallow upsert).
    UpdateNodeParams {
        /// Target node.
        id: Uuid,
        /// Key-value pairs to upsert.
        params: BTreeMap<String, serde_json::Value>,
    },
    /// Move a node to a new canvas position.
    MoveNode {
        /// Target node.
        id: Uuid,
        /// New `(x, y)` position.
        position: (f64, f64),
    },
    /// Insert a new link; fails if the ID already exists.
    AddLink(Link),
    /// Remove a link; fails if absent.
    RemoveLink {
        /// Link to remove.
        id: Uuid,
    },
    /// Insert a new group; fails if the ID already exists.
    AddGroup(Group),
    /// Remove a group; fails if absent.
    RemoveGroup {
        /// Group to remove.
        id: Uuid,
    },
    /// Add a node ID to a group.
    AddNodeToGroup {
        /// Node to include.
        node_id: Uuid,
        /// Target group.
        group_id: Uuid,
    },
    /// Remove a node ID from a group.
    RemoveNodeFromGroup {
        /// Node to exclude.
        node_id: Uuid,
        /// Target group.
        group_id: Uuid,
    },
    /// Insert a new subgraph; fails if the ID already exists.
    AddSubgraph(Subgraph),
    /// Remove a subgraph; fails if absent.
    RemoveSubgraph {
        /// Subgraph to remove.
        id: Uuid,
    },
}

/// An authored, time-stamped sequence of [`GraphPatchOp`]s.
///
/// Applied atomically (all-or-nothing) via [`FlowGraph::apply_patch`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPatch {
    /// Unique patch identifier.
    pub id: Uuid,
    /// Author of this patch.
    pub author: String,
    /// Unix epoch milliseconds when the patch was created.
    pub created_at_ms: u64,
    /// Ordered list of operations.
    pub ops: Vec<GraphPatchOp>,
}

impl GraphPatch {
    /// Create a new patch attributed to the given author.
    pub fn new(author: impl Into<String>, ops: Vec<GraphPatchOp>) -> Self {
        Self {
            id: Uuid::now_v7(),
            author: author.into(),
            created_at_ms: now_ms(),
            ops,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation report
// ---------------------------------------------------------------------------

/// A single validation issue found during [`FlowGraph::validate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Machine-readable issue kind.
    pub kind: String,
    /// Human-readable description.
    pub description: String,
    /// Node ID involved, if applicable.
    pub node_id: Option<Uuid>,
    /// Link ID involved, if applicable.
    pub link_id: Option<Uuid>,
}

/// Aggregated result of [`FlowGraph::validate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    /// `true` if no issues were found.
    pub valid: bool,
    /// List of all issues found.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    fn new() -> Self {
        Self { valid: true, issues: Vec::new() }
    }

    fn push(&mut self, issue: ValidationIssue) {
        self.valid = false;
        self.issues.push(issue);
    }
}

// ---------------------------------------------------------------------------
// FlowGraph — runtime wrapper
// ---------------------------------------------------------------------------

/// Runtime flow graph with full Epic F feature set.
///
/// Wraps a [`FlowGraphSchema`] and provides:
/// * node / link / group / subgraph mutation with ActionLog events,
/// * cycle detection and topological-order planning,
/// * type-compatibility validation,
/// * atomic patch application,
/// * compute diff between two schema snapshots.
pub struct FlowGraph {
    /// The underlying, serialisable graph data.
    pub schema: FlowGraphSchema,
    /// Per-node execution state, keyed by node ID.
    execution_states: HashMap<Uuid, NodeExecutionState>,
    action_log: Arc<ActionLog>,
    dimension_id: DimensionId,
    task_id: TaskId,
}

impl FlowGraph {
    /// Create a new, empty flow graph.
    pub fn new(
        dimension_id: DimensionId,
        task_id: TaskId,
        action_log: Arc<ActionLog>,
    ) -> Self {
        Self {
            schema: FlowGraphSchema::new(dimension_id),
            execution_states: HashMap::new(),
            action_log,
            dimension_id,
            task_id,
        }
    }

    // ── Node operations ──────────────────────────────────────────────────

    /// Add a node to the graph.
    ///
    /// Emits a [`EventType::NodeCreated`] ActionLog event.
    pub fn add_node(&mut self, node: GraphNode) -> Uuid {
        let id = node.id;
        self.schema.nodes.insert(id, node);
        self.execution_states.insert(id, NodeExecutionState::Idle);
        self.action_log.append(ActionLogEntry::new(
                EventType::NodeCreated,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "node_id": id }),
            ));
        id
    }

    /// Remove a node and all links that reference it.
    ///
    /// Emits a [`EventType::NodeDeleted`] ActionLog event.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::NodeNotFound`] if the node does not exist.
    pub fn remove_node(&mut self, id: Uuid) -> Result<GraphNode, FlowGraphError> {
        let node = self
            .schema
            .nodes
            .remove(&id)
            .ok_or(FlowGraphError::NodeNotFound(id))?;
        self.execution_states.remove(&id);
        // Remove all links referencing this node.
        self.schema
            .links
            .retain(|_, l| l.source_node != id && l.target_node != id);
        // Remove node from all groups.
        for group in self.schema.groups.values_mut() {
            group.node_ids.remove(&id);
        }
        self.action_log.append(ActionLogEntry::new(
                EventType::NodeDeleted,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "node_id": id }),
            ));
        Ok(node)
    }

    /// Return a reference to a node by ID.
    pub fn get_node(&self, id: Uuid) -> Option<&GraphNode> {
        self.schema.nodes.get(&id)
    }

    /// Return a mutable reference to a node by ID.
    pub fn get_node_mut(&mut self, id: Uuid) -> Option<&mut GraphNode> {
        self.schema.nodes.get_mut(&id)
    }

    // ── Link operations ──────────────────────────────────────────────────

    /// Add a directed link between two ports.
    ///
    /// Validates that both nodes and ports exist and that port data types are
    /// compatible before inserting.
    ///
    /// Emits a [`EventType::LinkCreated`] ActionLog event.
    ///
    /// # Errors
    ///
    /// * [`FlowGraphError::NodeNotFound`] — source or target node missing.
    /// * [`FlowGraphError::PortNotFound`] — port missing from its node.
    /// * [`FlowGraphError::TypeMismatch`] — incompatible port types.
    pub fn add_link(&mut self, link: Link) -> Result<Uuid, FlowGraphError> {
        let src_node = self
            .schema
            .nodes
            .get(&link.source_node)
            .ok_or(FlowGraphError::NodeNotFound(link.source_node))?;
        let src_port = src_node.ports.get(&link.source_port).ok_or(
            FlowGraphError::PortNotFound {
                node_id: link.source_node,
                port_id: link.source_port,
            },
        )?;
        let tgt_node = self
            .schema
            .nodes
            .get(&link.target_node)
            .ok_or(FlowGraphError::NodeNotFound(link.target_node))?;
        let tgt_port = tgt_node.ports.get(&link.target_port).ok_or(
            FlowGraphError::PortNotFound {
                node_id: link.target_node,
                port_id: link.target_port,
            },
        )?;
        if !src_port.data_type.is_compatible_with(tgt_port.data_type) {
            return Err(FlowGraphError::TypeMismatch {
                link_id: link.id,
                source_type: src_port.data_type,
                target_type: tgt_port.data_type,
            });
        }
        let id = link.id;
        self.schema.links.insert(id, link);
        self.action_log.append(ActionLogEntry::new(
                EventType::LinkCreated,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "link_id": id }),
            ));
        Ok(id)
    }

    /// Remove a link by ID.
    ///
    /// Emits a [`EventType::LinkRemoved`] ActionLog event.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::LinkNotFound`] if absent.
    pub fn remove_link(&mut self, id: Uuid) -> Result<Link, FlowGraphError> {
        let link = self
            .schema
            .links
            .remove(&id)
            .ok_or(FlowGraphError::LinkNotFound(id))?;
        self.action_log.append(ActionLogEntry::new(
                EventType::LinkRemoved,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "link_id": id }),
            ));
        Ok(link)
    }

    // ── Group operations ─────────────────────────────────────────────────

    /// Add a group to the graph.
    ///
    /// Emits a [`EventType::GroupCreated`] ActionLog event.
    pub fn add_group(&mut self, group: Group) -> Uuid {
        let id = group.id;
        self.schema.groups.insert(id, group);
        self.action_log.append(ActionLogEntry::new(
                EventType::GroupCreated,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "group_id": id }),
            ));
        id
    }

    /// Remove a group by ID.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::GroupNotFound`] if absent.
    pub fn remove_group(&mut self, id: Uuid) -> Result<Group, FlowGraphError> {
        let group = self
            .schema
            .groups
            .remove(&id)
            .ok_or(FlowGraphError::GroupNotFound(id))?;
        self.action_log.append(ActionLogEntry::new(
                EventType::GroupRemoved,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "group_id": id }),
            ));
        Ok(group)
    }

    // ── Subgraph operations ──────────────────────────────────────────────

    /// Register a subgraph / macro.
    ///
    /// Emits a [`EventType::SubgraphCreated`] ActionLog event.
    pub fn add_subgraph(&mut self, subgraph: Subgraph) -> Uuid {
        let id = subgraph.id;
        self.schema.subgraphs.insert(id, subgraph);
        self.action_log.append(ActionLogEntry::new(
                EventType::SubgraphCreated,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({ "subgraph_id": id }),
            ));
        id
    }

    /// Remove a subgraph by ID.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::SubgraphNotFound`] if absent.
    pub fn remove_subgraph(&mut self, id: Uuid) -> Result<Subgraph, FlowGraphError> {
        self.schema
            .subgraphs
            .remove(&id)
            .ok_or(FlowGraphError::SubgraphNotFound(id))
    }

    // ── Node relations ───────────────────────────────────────────────────

    /// Add a semantic [`NodeRelation`] between two nodes.
    ///
    /// Both endpoint nodes must already exist in the graph.
    ///
    /// Emits a [`EventType::NodeRelationCreated`] ActionLog event.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::NodeNotFound`] if either node is absent.
    pub fn add_relation(&mut self, relation: NodeRelation) -> Result<Uuid, FlowGraphError> {
        if !self.schema.nodes.contains_key(&relation.from_node) {
            return Err(FlowGraphError::NodeNotFound(relation.from_node));
        }
        if !self.schema.nodes.contains_key(&relation.to_node) {
            return Err(FlowGraphError::NodeNotFound(relation.to_node));
        }
        let id = relation.id;
        self.schema.relations.insert(id, relation);
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeRelationCreated,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "relation_id": id }),
        ));
        Ok(id)
    }

    /// Remove a relation by ID.
    ///
    /// Emits a [`EventType::NodeRelationRemoved`] ActionLog event.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::RelationNotFound`] if absent.
    pub fn remove_relation(&mut self, id: Uuid) -> Result<NodeRelation, FlowGraphError> {
        let rel = self
            .schema
            .relations
            .remove(&id)
            .ok_or(FlowGraphError::RelationNotFound(id))?;
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeRelationRemoved,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "relation_id": id }),
        ));
        Ok(rel)
    }

    /// Return all relations that involve `node_id` as either endpoint.
    pub fn relations_for_node(&self, node_id: Uuid) -> Vec<&NodeRelation> {
        self.schema
            .relations
            .values()
            .filter(|r| r.from_node == node_id || r.to_node == node_id)
            .collect()
    }

    /// Return all outgoing relations from `node_id` (where `from_node == node_id`).
    pub fn outgoing_relations(&self, node_id: Uuid) -> Vec<&NodeRelation> {
        self.schema
            .relations
            .values()
            .filter(|r| r.from_node == node_id)
            .collect()
    }

    /// Return all incoming relations to `node_id` (where `to_node == node_id`).
    pub fn incoming_relations(&self, node_id: Uuid) -> Vec<&NodeRelation> {
        self.schema
            .relations
            .values()
            .filter(|r| r.to_node == node_id)
            .collect()
    }

    /// Return the execution state for a node.
    pub fn execution_state(&self, node_id: Uuid) -> Option<&NodeExecutionState> {
        self.execution_states.get(&node_id)
    }

    /// Create an execution contract for a node.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::NodeNotFound`] if the node does not exist.
    pub fn execution_contract(
        &self,
        node_id: Uuid,
    ) -> Result<NodeExecutionContract, FlowGraphError> {
        if !self.schema.nodes.contains_key(&node_id) {
            return Err(FlowGraphError::NodeNotFound(node_id));
        }
        Ok(NodeExecutionContract::new(
            node_id,
            self.dimension_id,
            self.task_id,
            Arc::clone(&self.action_log),
        ))
    }

    // ── Cycle detection ──────────────────────────────────────────────────

    /// Detect whether the graph contains a directed cycle.
    ///
    /// Uses depth-first search with a recursion-stack colour (white / grey /
    /// black).  Returns the cycle as a sequence of node IDs if found, or
    /// `Ok(())` if the graph is acyclic.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::CycleDetected`] containing the cycle path.
    pub fn detect_cycle(&self) -> Result<(), FlowGraphError> {
        let adj = self.adjacency_list();
        let ids: Vec<Uuid> = self.schema.nodes.keys().copied().collect();

        // 0 = unvisited (white), 1 = in stack (grey), 2 = done (black)
        let mut colour: HashMap<Uuid, u8> = ids.iter().map(|&id| (id, 0u8)).collect();
        let mut stack: Vec<Uuid> = Vec::new();

        for &start in &ids {
            if colour[&start] == 0 {
                self.dfs_cycle(start, &adj, &mut colour, &mut stack)?;
            }
        }
        Ok(())
    }

    fn dfs_cycle(
        &self,
        node: Uuid,
        adj: &HashMap<Uuid, Vec<Uuid>>,
        colour: &mut HashMap<Uuid, u8>,
        stack: &mut Vec<Uuid>,
    ) -> Result<(), FlowGraphError> {
        colour.insert(node, 1);
        stack.push(node);

        if let Some(neighbours) = adj.get(&node) {
            for &next in neighbours {
                match colour.get(&next).copied().unwrap_or(0) {
                    1 => {
                        // Back edge → cycle found; extract cycle path.
                        let cycle_start = stack.iter().position(|&n| n == next).unwrap_or(0);
                        let mut cycle = stack[cycle_start..].to_vec();
                        cycle.push(next);
                        return Err(FlowGraphError::CycleDetected(cycle));
                    }
                    0 => self.dfs_cycle(next, adj, colour, stack)?,
                    _ => {}
                }
            }
        }

        stack.pop();
        colour.insert(node, 2);
        Ok(())
    }

    // ── Topological order ────────────────────────────────────────────────

    /// Compute a deterministic execution order using Kahn's algorithm
    /// (BFS-based topological sort).
    ///
    /// Returns a `Vec` of node IDs ordered so that every dependency appears
    /// before the nodes that consume it.  Nodes at the same depth are sorted
    /// by ID to guarantee determinism across runs.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::CycleDetected`] if the graph is not a DAG.
    pub fn topological_order(&self) -> Result<Vec<Uuid>, FlowGraphError> {
        let adj = self.adjacency_list();

        // Compute in-degree for every node.
        let mut in_degree: BTreeMap<Uuid, usize> =
            self.schema.nodes.keys().map(|&id| (id, 0)).collect();
        for links in adj.values() {
            for &tgt in links {
                *in_degree.entry(tgt).or_insert(0) += 1;
            }
        }

        // Seed queue with nodes that have no incoming edges (sorted for
        // determinism).
        let mut queue: VecDeque<Uuid> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();
        // Sort initial batch for determinism.
        let mut sorted_queue: Vec<Uuid> = queue.drain(..).collect();
        sorted_queue.sort_unstable();
        queue.extend(sorted_queue);

        let mut order: Vec<Uuid> = Vec::with_capacity(self.schema.nodes.len());

        while let Some(node) = queue.pop_front() {
            order.push(node);
            if let Some(neighbours) = adj.get(&node) {
                let mut batch: Vec<Uuid> = Vec::new();
                for &next in neighbours {
                    let deg = in_degree.entry(next).or_insert(0);
                    *deg -= 1;
                    if *deg == 0 {
                        batch.push(next);
                    }
                }
                // Sort each batch for determinism.
                batch.sort_unstable();
                queue.extend(batch);
            }
        }

        if order.len() != self.schema.nodes.len() {
            // Some nodes were never dequeued → cycle present.
            let cycle_nodes: Vec<Uuid> = self
                .schema
                .nodes
                .keys()
                .filter(|id| !order.contains(id))
                .copied()
                .collect();
            return Err(FlowGraphError::CycleDetected(cycle_nodes));
        }

        Ok(order)
    }

    // ── Validation ───────────────────────────────────────────────────────

    /// Validate the graph and return a [`ValidationReport`].
    ///
    /// Checks performed:
    /// 1. All link endpoints refer to existing nodes and ports.
    /// 2. Source port direction is `Out` and target port direction is `In`.
    /// 3. Port data-type compatibility.
    /// 4. Required input ports have at least one incoming link.
    /// 5. No directed cycles (via [`Self::detect_cycle`]).
    ///
    /// Emits a [`EventType::GraphValidated`] ActionLog event.
    pub fn validate(&self) -> ValidationReport {
        let mut report = ValidationReport::new();

        // Build a map: (node_id, port_id) → incoming link count.
        let mut incoming: HashMap<(Uuid, Uuid), usize> = HashMap::new();

        for (&link_id, link) in &self.schema.links {
            // Check source node + port.
            let src_port_opt = self
                .schema
                .nodes
                .get(&link.source_node)
                .and_then(|n| n.ports.get(&link.source_port));

            // Check target node + port.
            let tgt_port_opt = self
                .schema
                .nodes
                .get(&link.target_node)
                .and_then(|n| n.ports.get(&link.target_port));

            match (src_port_opt, tgt_port_opt) {
                (Some(src), Some(tgt)) => {
                    // Direction checks.
                    if src.direction != PortDirection::Out {
                        report.push(ValidationIssue {
                            kind: "wrong_source_port_direction".into(),
                            description: format!(
                                "Link {link_id}: source port '{}' is not an Out port",
                                src.name
                            ),
                            node_id: Some(link.source_node),
                            link_id: Some(link_id),
                        });
                    }
                    if tgt.direction != PortDirection::In {
                        report.push(ValidationIssue {
                            kind: "wrong_target_port_direction".into(),
                            description: format!(
                                "Link {link_id}: target port '{}' is not an In port",
                                tgt.name
                            ),
                            node_id: Some(link.target_node),
                            link_id: Some(link_id),
                        });
                    }
                    // Type compatibility.
                    if !src.data_type.is_compatible_with(tgt.data_type) {
                        report.push(ValidationIssue {
                            kind: "type_mismatch".into(),
                            description: format!(
                                "Link {link_id}: type {:?} ↛ {:?}",
                                src.data_type, tgt.data_type
                            ),
                            node_id: None,
                            link_id: Some(link_id),
                        });
                    }
                    // Count incoming link for required-port check.
                    *incoming
                        .entry((link.target_node, link.target_port))
                        .or_insert(0) += 1;
                }
                (None, _) => {
                    if !self.schema.nodes.contains_key(&link.source_node) {
                        report.push(ValidationIssue {
                            kind: "missing_source_node".into(),
                            description: format!(
                                "Link {link_id}: source node {} not found",
                                link.source_node
                            ),
                            node_id: Some(link.source_node),
                            link_id: Some(link_id),
                        });
                    } else {
                        report.push(ValidationIssue {
                            kind: "missing_source_port".into(),
                            description: format!(
                                "Link {link_id}: source port {} not found on node {}",
                                link.source_port, link.source_node
                            ),
                            node_id: Some(link.source_node),
                            link_id: Some(link_id),
                        });
                    }
                }
                (_, None) => {
                    if !self.schema.nodes.contains_key(&link.target_node) {
                        report.push(ValidationIssue {
                            kind: "missing_target_node".into(),
                            description: format!(
                                "Link {link_id}: target node {} not found",
                                link.target_node
                            ),
                            node_id: Some(link.target_node),
                            link_id: Some(link_id),
                        });
                    } else {
                        report.push(ValidationIssue {
                            kind: "missing_target_port".into(),
                            description: format!(
                                "Link {link_id}: target port {} not found on node {}",
                                link.target_port, link.target_node
                            ),
                            node_id: Some(link.target_node),
                            link_id: Some(link_id),
                        });
                    }
                }
            }
        }

        // Required-port check.
        for (&node_id, node) in &self.schema.nodes {
            for port in node.inputs().filter(|p| p.required) {
                if incoming.get(&(node_id, port.id)).copied().unwrap_or(0) == 0 {
                    report.push(ValidationIssue {
                        kind: "unconnected_required_port".into(),
                        description: format!(
                            "Node {node_id}: required input port '{}' has no incoming link",
                            port.name
                        ),
                        node_id: Some(node_id),
                        link_id: None,
                    });
                }
            }
        }

        // Cycle check.
        if let Err(FlowGraphError::CycleDetected(cycle)) = self.detect_cycle() {
            report.push(ValidationIssue {
                kind: "cycle_detected".into(),
                description: format!("Directed cycle detected through nodes: {cycle:?}"),
                node_id: None,
                link_id: None,
            });
        }

        self.action_log.append(ActionLogEntry::new(
                EventType::GraphValidated,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({
                "valid": report.valid,
                "issue_count": report.issues.len(),
            }),
            ));

        report
    }

    // ── Patch application ────────────────────────────────────────────────

    /// Apply a [`GraphPatch`] atomically (all operations or none).
    ///
    /// Operations are applied sequentially to a working copy; on the first
    /// error the working copy is discarded and the original schema is
    /// preserved.
    ///
    /// Emits a [`EventType::GraphPatched`] ActionLog event on success.
    ///
    /// # Errors
    ///
    /// Returns [`FlowGraphError::InvalidPatch`] wrapping the first failing
    /// operation's error message.
    pub fn apply_patch(&mut self, patch: &GraphPatch) -> Result<(), FlowGraphError> {
        // Work on a clone; commit only on full success.
        let mut working = self.schema.clone();

        for op in &patch.ops {
            apply_op_to_schema(&mut working, op).map_err(|e| {
                FlowGraphError::InvalidPatch(format!(
                    "patch {} op failed: {e}",
                    patch.id
                ))
            })?;
        }

        self.schema = working;
        self.action_log.append(ActionLogEntry::new(
                EventType::GraphPatched,
                Actor::System,
                Some(self.dimension_id),
                Some(self.task_id),
                serde_json::json!({
                "patch_id": patch.id,
                "author": patch.author,
                "op_count": patch.ops.len(),
            }),
            ));
        Ok(())
    }

    // ── Diff ─────────────────────────────────────────────────────────────

    /// Compute the [`GraphPatch`] needed to transform `from` into `to`.
    ///
    /// Produced operations: added/removed nodes, added/removed links,
    /// added/removed groups.
    pub fn compute_diff(
        from: &FlowGraphSchema,
        to: &FlowGraphSchema,
        author: impl Into<String>,
    ) -> GraphPatch {
        let mut ops: Vec<GraphPatchOp> = Vec::new();

        // Nodes added.
        for (&id, node) in &to.nodes {
            if !from.nodes.contains_key(&id) {
                ops.push(GraphPatchOp::AddNode(node.clone()));
            }
        }
        // Nodes removed.
        for &id in from.nodes.keys() {
            if !to.nodes.contains_key(&id) {
                ops.push(GraphPatchOp::RemoveNode { id });
            }
        }
        // Links added.
        for (&id, link) in &to.links {
            if !from.links.contains_key(&id) {
                ops.push(GraphPatchOp::AddLink(link.clone()));
            }
        }
        // Links removed.
        for &id in from.links.keys() {
            if !to.links.contains_key(&id) {
                ops.push(GraphPatchOp::RemoveLink { id });
            }
        }
        // Groups added.
        for (&id, group) in &to.groups {
            if !from.groups.contains_key(&id) {
                ops.push(GraphPatchOp::AddGroup(group.clone()));
            }
        }
        // Groups removed.
        for &id in from.groups.keys() {
            if !to.groups.contains_key(&id) {
                ops.push(GraphPatchOp::RemoveGroup { id });
            }
        }
        // Subgraphs added.
        for (&id, sg) in &to.subgraphs {
            if !from.subgraphs.contains_key(&id) {
                ops.push(GraphPatchOp::AddSubgraph(sg.clone()));
            }
        }
        // Subgraphs removed.
        for &id in from.subgraphs.keys() {
            if !to.subgraphs.contains_key(&id) {
                ops.push(GraphPatchOp::RemoveSubgraph { id });
            }
        }

        GraphPatch::new(author, ops)
    }

    // ── Internal helpers ─────────────────────────────────────────────────

    /// Build an adjacency list (source → Vec<target>) from the current links.
    fn adjacency_list(&self) -> HashMap<Uuid, Vec<Uuid>> {
        let mut adj: HashMap<Uuid, Vec<Uuid>> = self
            .schema
            .nodes
            .keys()
            .map(|&id| (id, Vec::new()))
            .collect();
        for link in self.schema.links.values() {
            adj.entry(link.source_node).or_default().push(link.target_node);
        }
        adj
    }
}

// ---------------------------------------------------------------------------
// Internal: apply a single patch op to a schema (no logging)
// ---------------------------------------------------------------------------

fn apply_op_to_schema(
    schema: &mut FlowGraphSchema,
    op: &GraphPatchOp,
) -> Result<(), FlowGraphError> {
    match op {
        GraphPatchOp::AddNode(node) => {
            if schema.nodes.contains_key(&node.id) {
                return Err(FlowGraphError::InvalidPatch(format!(
                    "node {} already exists",
                    node.id
                )));
            }
            schema.nodes.insert(node.id, node.clone());
        }
        GraphPatchOp::RemoveNode { id } => {
            schema
                .nodes
                .remove(id)
                .ok_or(FlowGraphError::NodeNotFound(*id))?;
            schema.links.retain(|_, l| l.source_node != *id && l.target_node != *id);
            for g in schema.groups.values_mut() {
                g.node_ids.remove(id);
            }
        }
        GraphPatchOp::UpdateNodeLabel { id, new_label } => {
            let node = schema.nodes.get_mut(id).ok_or(FlowGraphError::NodeNotFound(*id))?;
            node.label = new_label.clone();
        }
        GraphPatchOp::UpdateNodeParams { id, params } => {
            let node = schema.nodes.get_mut(id).ok_or(FlowGraphError::NodeNotFound(*id))?;
            for (k, v) in params {
                node.parameters.insert(k.clone(), v.clone());
            }
        }
        GraphPatchOp::MoveNode { id, position } => {
            let node = schema.nodes.get_mut(id).ok_or(FlowGraphError::NodeNotFound(*id))?;
            node.position = *position;
        }
        GraphPatchOp::AddLink(link) => {
            if schema.links.contains_key(&link.id) {
                return Err(FlowGraphError::InvalidPatch(format!(
                    "link {} already exists",
                    link.id
                )));
            }
            schema.links.insert(link.id, link.clone());
        }
        GraphPatchOp::RemoveLink { id } => {
            schema
                .links
                .remove(id)
                .ok_or(FlowGraphError::LinkNotFound(*id))?;
        }
        GraphPatchOp::AddGroup(group) => {
            if schema.groups.contains_key(&group.id) {
                return Err(FlowGraphError::InvalidPatch(format!(
                    "group {} already exists",
                    group.id
                )));
            }
            schema.groups.insert(group.id, group.clone());
        }
        GraphPatchOp::RemoveGroup { id } => {
            schema
                .groups
                .remove(id)
                .ok_or(FlowGraphError::GroupNotFound(*id))?;
        }
        GraphPatchOp::AddNodeToGroup { node_id, group_id } => {
            let group = schema
                .groups
                .get_mut(group_id)
                .ok_or(FlowGraphError::GroupNotFound(*group_id))?;
            group.node_ids.insert(*node_id);
        }
        GraphPatchOp::RemoveNodeFromGroup { node_id, group_id } => {
            let group = schema
                .groups
                .get_mut(group_id)
                .ok_or(FlowGraphError::GroupNotFound(*group_id))?;
            group.node_ids.remove(node_id);
        }
        GraphPatchOp::AddSubgraph(sg) => {
            if schema.subgraphs.contains_key(&sg.id) {
                return Err(FlowGraphError::InvalidPatch(format!(
                    "subgraph {} already exists",
                    sg.id
                )));
            }
            schema.subgraphs.insert(sg.id, sg.clone());
        }
        GraphPatchOp::RemoveSubgraph { id } => {
            schema
                .subgraphs
                .remove(id)
                .ok_or(FlowGraphError::SubgraphNotFound(*id))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — including graph execution determinism harness (Epic F item 11)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Fixtures ─────────────────────────────────────────────────────────

    fn make_graph() -> FlowGraph {
        let log = ActionLog::new(64);
        let dim = DimensionId::new();
        let task = TaskId::new();
        FlowGraph::new(dim, task, log)
    }

    fn num_out_port() -> PortDef {
        PortDef::new("value", PortDirection::Out, PortDataType::Number)
    }

    fn num_in_port() -> PortDef {
        PortDef::new("input", PortDirection::In, PortDataType::Number)
    }

    fn make_node_with_ports(kind: &str) -> GraphNode {
        let mut node = GraphNode::new(kind, kind);
        node.add_port(num_out_port()).unwrap();
        node.add_port(num_in_port()).unwrap();
        node
    }

    // ── Port compatibility ────────────────────────────────────────────────

    #[test]
    fn port_any_compatible_with_all() {
        assert!(PortDataType::Any.is_compatible_with(PortDataType::Number));
        assert!(PortDataType::Number.is_compatible_with(PortDataType::Any));
        assert!(PortDataType::Any.is_compatible_with(PortDataType::Json));
    }

    #[test]
    fn port_same_types_compatible() {
        assert!(PortDataType::Number.is_compatible_with(PortDataType::Number));
        assert!(PortDataType::String.is_compatible_with(PortDataType::String));
    }

    #[test]
    fn port_different_types_incompatible() {
        assert!(!PortDataType::Number.is_compatible_with(PortDataType::String));
        assert!(!PortDataType::Bool.is_compatible_with(PortDataType::Json));
    }

    // ── Node operations ──────────────────────────────────────────────────

    #[test]
    fn add_and_remove_node() {
        let mut g = make_graph();
        let node = GraphNode::new("test", "Test");
        let id = g.add_node(node);
        assert!(g.get_node(id).is_some());
        g.remove_node(id).unwrap();
        assert!(g.get_node(id).is_none());
    }

    #[test]
    fn remove_nonexistent_node_fails() {
        let mut g = make_graph();
        let err = g.remove_node(Uuid::new_v4());
        assert!(matches!(err, Err(FlowGraphError::NodeNotFound(_))));
    }

    #[test]
    fn duplicate_port_name_rejected() {
        let mut node = GraphNode::new("dup", "Dup");
        node.add_port(PortDef::new("x", PortDirection::Out, PortDataType::Number))
            .unwrap();
        let err = node
            .add_port(PortDef::new("x", PortDirection::Out, PortDataType::Number))
            .unwrap_err();
        assert!(matches!(err, FlowGraphError::DuplicatePortName { .. }));
    }

    // ── Link operations ──────────────────────────────────────────────────

    #[test]
    fn add_valid_link() {
        let mut g = make_graph();
        let mut src = GraphNode::new("src", "Source");
        let out_port = num_out_port();
        let out_id = src.add_port(out_port).unwrap();
        let src_id = g.add_node(src);

        let mut tgt = GraphNode::new("tgt", "Target");
        let in_port = num_in_port();
        let in_id = tgt.add_port(in_port).unwrap();
        let tgt_id = g.add_node(tgt);

        let link = Link::new(src_id, out_id, tgt_id, in_id);
        let link_id = g.add_link(link).unwrap();
        assert!(g.schema.links.contains_key(&link_id));
    }

    #[test]
    fn add_link_type_mismatch_rejected() {
        let mut g = make_graph();
        let mut src = GraphNode::new("src", "Source");
        let out_port = PortDef::new("val", PortDirection::Out, PortDataType::Number);
        let out_id = src.add_port(out_port).unwrap();
        let src_id = g.add_node(src);

        let mut tgt = GraphNode::new("tgt", "Target");
        let in_port = PortDef::new("in", PortDirection::In, PortDataType::String);
        let in_id = tgt.add_port(in_port).unwrap();
        let tgt_id = g.add_node(tgt);

        let link = Link::new(src_id, out_id, tgt_id, in_id);
        let err = g.add_link(link).unwrap_err();
        assert!(matches!(err, FlowGraphError::TypeMismatch { .. }));
    }

    #[test]
    fn remove_node_cascades_links() {
        let mut g = make_graph();
        let mut src = GraphNode::new("src", "S");
        let out_id = src.add_port(num_out_port()).unwrap();
        let src_id = g.add_node(src);

        let mut tgt = GraphNode::new("tgt", "T");
        let in_id = tgt.add_port(num_in_port()).unwrap();
        let tgt_id = g.add_node(tgt);

        let link = Link::new(src_id, out_id, tgt_id, in_id);
        let link_id = g.add_link(link).unwrap();

        g.remove_node(src_id).unwrap();
        assert!(!g.schema.links.contains_key(&link_id));
    }

    // ── Cycle detection ──────────────────────────────────────────────────

    #[test]
    fn acyclic_graph_passes() {
        let mut g = make_graph();
        let mut a = GraphNode::new("A", "A");
        let a_out = a.add_port(num_out_port()).unwrap();
        let a_id = g.add_node(a);

        let mut b = GraphNode::new("B", "B");
        let b_in = b.add_port(num_in_port()).unwrap();
        b.add_port(num_out_port()).unwrap();
        let b_id = g.add_node(b);

        let link = Link::new(a_id, a_out, b_id, b_in);
        g.add_link(link).unwrap();

        assert!(g.detect_cycle().is_ok());
    }

    #[test]
    fn cyclic_graph_detected() {
        let mut g = make_graph();
        // Build A → B → C → A (using Any ports to avoid type errors).
        let make_any_node = |kind: &str| {
            let mut n = GraphNode::new(kind, kind);
            n.add_port(PortDef::new("out", PortDirection::Out, PortDataType::Any)).unwrap();
            n.add_port(PortDef::new("in", PortDirection::In, PortDataType::Any)).unwrap();
            n
        };

        let mut a = make_any_node("A");
        let a_out = a.ports.values().find(|p| p.direction == PortDirection::Out).unwrap().id;
        let a_in = a.ports.values().find(|p| p.direction == PortDirection::In).unwrap().id;
        let a_id = g.add_node(a);

        let mut b = make_any_node("B");
        let b_out = b.ports.values().find(|p| p.direction == PortDirection::Out).unwrap().id;
        let b_in = b.ports.values().find(|p| p.direction == PortDirection::In).unwrap().id;
        let b_id = g.add_node(b);

        let mut c = make_any_node("C");
        let c_out = c.ports.values().find(|p| p.direction == PortDirection::Out).unwrap().id;
        let c_in = c.ports.values().find(|p| p.direction == PortDirection::In).unwrap().id;
        let c_id = g.add_node(c);

        g.add_link(Link::new(a_id, a_out, b_id, b_in)).unwrap();
        g.add_link(Link::new(b_id, b_out, c_id, c_in)).unwrap();
        g.add_link(Link::new(c_id, c_out, a_id, a_in)).unwrap();

        assert!(matches!(g.detect_cycle(), Err(FlowGraphError::CycleDetected(_))));
    }

    // ── Topological order ────────────────────────────────────────────────

    #[test]
    fn topological_order_linear_chain() {
        let mut g = make_graph();
        let make_any_node = |kind: &str| {
            let mut n = GraphNode::new(kind, kind);
            n.add_port(PortDef::new("out", PortDirection::Out, PortDataType::Any)).unwrap();
            n.add_port(PortDef::new("in", PortDirection::In, PortDataType::Any)).unwrap();
            n
        };

        let mut a = make_any_node("A");
        let a_out = a.ports.values().find(|p| p.direction == PortDirection::Out).unwrap().id;
        let a_id = g.add_node(a);

        let mut b = make_any_node("B");
        let b_out = b.ports.values().find(|p| p.direction == PortDirection::Out).unwrap().id;
        let b_in = b.ports.values().find(|p| p.direction == PortDirection::In).unwrap().id;
        let b_id = g.add_node(b);

        let mut c = make_any_node("C");
        let c_in = c.ports.values().find(|p| p.direction == PortDirection::In).unwrap().id;
        let c_id = g.add_node(c);

        g.add_link(Link::new(a_id, a_out, b_id, b_in)).unwrap();
        g.add_link(Link::new(b_id, b_out, c_id, c_in)).unwrap();

        let order = g.topological_order().unwrap();
        assert_eq!(order.len(), 3);
        let ai = order.iter().position(|&x| x == a_id).unwrap();
        let bi = order.iter().position(|&x| x == b_id).unwrap();
        let ci = order.iter().position(|&x| x == c_id).unwrap();
        assert!(ai < bi, "A must come before B");
        assert!(bi < ci, "B must come before C");
    }

    #[test]
    fn topological_order_is_deterministic() {
        // Build the same graph twice and verify that `topological_order()`
        // returns an identical sequence — this is the graph execution
        // determinism harness.
        let build_graph = || {
            let log = ActionLog::new(32);
            let dim = DimensionId::from_uuid(
                Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            );
            let task = TaskId::from_uuid(
                Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            );
            let mut g = FlowGraph::new(dim, task, log);

            let node_uuids = [
                Uuid::parse_str("10000000-0000-0000-0000-000000000001").unwrap(),
                Uuid::parse_str("10000000-0000-0000-0000-000000000002").unwrap(),
                Uuid::parse_str("10000000-0000-0000-0000-000000000003").unwrap(),
            ];
            let port_uuids = [
                Uuid::parse_str("20000000-0000-0000-0000-000000000001").unwrap(),
                Uuid::parse_str("20000000-0000-0000-0000-000000000002").unwrap(),
                Uuid::parse_str("20000000-0000-0000-0000-000000000003").unwrap(),
                Uuid::parse_str("20000000-0000-0000-0000-000000000004").unwrap(),
            ];

            // Node A: out port only
            let mut a = GraphNode::new("A", "A");
            a.id = node_uuids[0];
            let mut pa_out = PortDef::new("out", PortDirection::Out, PortDataType::Any);
            pa_out.id = port_uuids[0];
            a.add_port(pa_out).unwrap();
            g.add_node(a);

            // Node B: in + out ports
            let mut b = GraphNode::new("B", "B");
            b.id = node_uuids[1];
            let mut pb_in = PortDef::new("in", PortDirection::In, PortDataType::Any);
            pb_in.id = port_uuids[1];
            let mut pb_out = PortDef::new("out", PortDirection::Out, PortDataType::Any);
            pb_out.id = port_uuids[2];
            b.add_port(pb_in).unwrap();
            b.add_port(pb_out).unwrap();
            g.add_node(b);

            // Node C: in port only
            let mut c = GraphNode::new("C", "C");
            c.id = node_uuids[2];
            let mut pc_in = PortDef::new("in", PortDirection::In, PortDataType::Any);
            pc_in.id = port_uuids[3];
            c.add_port(pc_in).unwrap();
            g.add_node(c);

            // Links: A→B, B→C
            let mut l1 = Link::new(node_uuids[0], port_uuids[0], node_uuids[1], port_uuids[1]);
            l1.id = Uuid::parse_str("30000000-0000-0000-0000-000000000001").unwrap();
            g.add_link(l1).unwrap();

            let mut l2 = Link::new(node_uuids[1], port_uuids[2], node_uuids[2], port_uuids[3]);
            l2.id = Uuid::parse_str("30000000-0000-0000-0000-000000000002").unwrap();
            g.add_link(l2).unwrap();

            g
        };

        let order1 = build_graph().topological_order().unwrap();
        let order2 = build_graph().topological_order().unwrap();
        assert_eq!(order1, order2, "topological order must be deterministic");
    }

    // ── Execution contracts ───────────────────────────────────────────────

    #[test]
    fn execution_contract_happy_path() {
        let mut g = make_graph();
        let node = GraphNode::new("worker", "Worker");
        let id = g.add_node(node);

        let mut contract = g.execution_contract(id).unwrap();
        contract.start().unwrap();
        contract.progress(50.0, "halfway").unwrap();
        contract.complete(serde_json::json!({ "result": 42 })).unwrap();

        assert!(contract.state().is_terminal());
        assert!(matches!(contract.state(), NodeExecutionState::Complete { .. }));
    }

    #[test]
    fn execution_contract_fail_path() {
        let mut g = make_graph();
        let node = GraphNode::new("worker", "Worker");
        let id = g.add_node(node);

        let mut contract = g.execution_contract(id).unwrap();
        contract.start().unwrap();
        contract.fail("something went wrong").unwrap();

        assert!(matches!(contract.state(), NodeExecutionState::Failed { .. }));
    }

    #[test]
    fn execution_contract_cancel_path() {
        let mut g = make_graph();
        let node = GraphNode::new("worker", "Worker");
        let id = g.add_node(node);

        let mut contract = g.execution_contract(id).unwrap();
        contract.start().unwrap();
        contract.cancel().unwrap();

        assert!(matches!(contract.state(), NodeExecutionState::Cancelled { .. }));
    }

    #[test]
    fn execution_contract_invalid_transition() {
        let mut g = make_graph();
        let node = GraphNode::new("worker", "Worker");
        let id = g.add_node(node);

        let mut contract = g.execution_contract(id).unwrap();
        // Can't complete from Idle.
        let err = contract.complete(serde_json::json!({})).unwrap_err();
        assert!(matches!(err, FlowGraphError::InvalidStateTransition { .. }));
    }

    // ── Graph validation ─────────────────────────────────────────────────

    #[test]
    fn validate_clean_graph_passes() {
        let g = make_graph();
        let report = g.validate();
        assert!(report.valid);
        assert!(report.issues.is_empty());
    }

    #[test]
    fn validate_unconnected_required_port_fails() {
        let mut g = make_graph();
        let mut node = GraphNode::new("sink", "Sink");
        let req_port = PortDef::new("required_in", PortDirection::In, PortDataType::Number)
            .required();
        node.add_port(req_port).unwrap();
        g.add_node(node);

        let report = g.validate();
        assert!(!report.valid);
        assert!(report.issues.iter().any(|i| i.kind == "unconnected_required_port"));
    }

    #[test]
    fn validate_cycle_reported() {
        let mut g = make_graph();
        let make_any_node = |kind: &str| {
            let mut n = GraphNode::new(kind, kind);
            n.add_port(PortDef::new("out", PortDirection::Out, PortDataType::Any)).unwrap();
            n.add_port(PortDef::new("in", PortDirection::In, PortDataType::Any)).unwrap();
            n
        };

        let mut a = make_any_node("A");
        let a_out = a.ports.values().find(|p| p.direction == PortDirection::Out).unwrap().id;
        let a_in = a.ports.values().find(|p| p.direction == PortDirection::In).unwrap().id;
        let a_id = g.add_node(a);

        let mut b = make_any_node("B");
        let b_out = b.ports.values().find(|p| p.direction == PortDirection::Out).unwrap().id;
        let b_in = b.ports.values().find(|p| p.direction == PortDirection::In).unwrap().id;
        let b_id = g.add_node(b);

        g.add_link(Link::new(a_id, a_out, b_id, b_in)).unwrap();
        g.add_link(Link::new(b_id, b_out, a_id, a_in)).unwrap();

        let report = g.validate();
        assert!(!report.valid);
        assert!(report.issues.iter().any(|i| i.kind == "cycle_detected"));
    }

    // ── Serialisation determinism ─────────────────────────────────────────

    #[test]
    fn serialisation_is_deterministic() {
        let dim = DimensionId::from_uuid(
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        );
        let make_schema = || {
            let mut s = FlowGraphSchema {
                schema_version: GRAPH_SCHEMA_VERSION,
                graph_id: Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap(),
                dimension_id: dim,
                nodes: BTreeMap::new(),
                links: BTreeMap::new(),
                groups: BTreeMap::new(),
                subgraphs: BTreeMap::new(),
                relations: BTreeMap::new(),
            };
            let mut node = GraphNode::new("A", "A");
            node.id = Uuid::parse_str("10000000-0000-0000-0000-000000000001").unwrap();
            node.parameters.insert("k".into(), serde_json::json!(1));
            // Pin the provenance timestamp so the JSON output is stable.
            node.provenance = NodeProvenance {
                created_by: "system".into(),
                created_at_ms: 0,
                modified_by: None,
                modified_at_ms: None,
                reason: None,
                task_id: None,
            };
            s.nodes.insert(node.id, node);
            s
        };

        let s1 = make_schema().to_json().unwrap();
        let s2 = make_schema().to_json().unwrap();
        assert_eq!(s1, s2, "identical schemas must produce identical JSON");
    }

    #[test]
    fn schema_roundtrip() {
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);
        let json = schema.to_json().unwrap();
        let restored = FlowGraphSchema::from_json(&json).unwrap();
        assert_eq!(schema.graph_id, restored.graph_id);
        assert_eq!(schema.schema_version, restored.schema_version);
    }

    #[test]
    fn schema_version_mismatch_rejected() {
        let dim = DimensionId::new();
        let mut schema = FlowGraphSchema::new(dim);
        schema.schema_version = 999;
        let json = serde_json::to_string(&schema).unwrap();
        let err = FlowGraphSchema::from_json(&json).unwrap_err();
        assert!(matches!(err, FlowGraphError::SchemaMismatch { found: 999, .. }));
    }

    // ── Patch application ─────────────────────────────────────────────────

    #[test]
    fn patch_add_and_remove_node() {
        let mut g = make_graph();
        let node = GraphNode::new("patched", "Patched");
        let node_id = node.id;

        let patch_add =
            GraphPatch::new("test", vec![GraphPatchOp::AddNode(node)]);
        g.apply_patch(&patch_add).unwrap();
        assert!(g.schema.nodes.contains_key(&node_id));

        let patch_remove =
            GraphPatch::new("test", vec![GraphPatchOp::RemoveNode { id: node_id }]);
        g.apply_patch(&patch_remove).unwrap();
        assert!(!g.schema.nodes.contains_key(&node_id));
    }

    #[test]
    fn patch_atomic_rollback_on_error() {
        let mut g = make_graph();
        let node = GraphNode::new("existing", "Existing");
        let node_id = node.id;
        g.add_node(node.clone());

        // The patch tries to add `node` again (duplicate) after a valid op.
        let ops = vec![
            GraphPatchOp::AddNode(GraphNode::new("valid_new", "Valid New")),
            GraphPatchOp::AddNode(node), // duplicate — should fail
        ];
        let patch = GraphPatch::new("test", ops);
        let err = g.apply_patch(&patch);
        assert!(err.is_err());

        // The graph must still contain only the original node.
        assert_eq!(g.schema.nodes.len(), 1);
        assert!(g.schema.nodes.contains_key(&node_id));
    }

    // ── Diff ─────────────────────────────────────────────────────────────

    #[test]
    fn diff_captures_added_and_removed_nodes() {
        let dim = DimensionId::new();
        let from = FlowGraphSchema::new(dim);
        let mut to = FlowGraphSchema {
            schema_version: GRAPH_SCHEMA_VERSION,
            graph_id: from.graph_id,
            dimension_id: from.dimension_id,
            nodes: from.nodes.clone(),
            links: from.links.clone(),
            groups: from.groups.clone(),
            subgraphs: from.subgraphs.clone(),
            relations: from.relations.clone(),
        };
        let new_node = GraphNode::new("added", "Added");
        let new_id = new_node.id;
        to.nodes.insert(new_id, new_node);

        let patch = FlowGraph::compute_diff(&from, &to, "agent");
        assert!(patch.ops.iter().any(|op| matches!(op, GraphPatchOp::AddNode(n) if n.id == new_id)));

        // Applying the patch to `from` should yield `to`.
        let mut g = FlowGraph::new(dim, TaskId::new(), ActionLog::new(16));
        g.schema = from.clone();
        g.apply_patch(&patch).unwrap();
        assert!(g.schema.nodes.contains_key(&new_id));
    }

    // ── Subgraph ──────────────────────────────────────────────────────────

    #[test]
    fn add_and_remove_subgraph() {
        let mut g = make_graph();
        let inner = FlowGraphSchema::new(g.schema.dimension_id);
        let sg = Subgraph::new("MySub", inner);
        let sg_id = sg.id;

        g.add_subgraph(sg);
        assert!(g.schema.subgraphs.contains_key(&sg_id));

        g.remove_subgraph(sg_id).unwrap();
        assert!(!g.schema.subgraphs.contains_key(&sg_id));
    }

    // ── Group operations ──────────────────────────────────────────────────

    #[test]
    fn group_membership() {
        let mut g = make_graph();
        let node = GraphNode::new("n", "N");
        let node_id = g.add_node(node);

        let group = Group::new("MyGroup");
        let group_id = g.add_group(group);

        let patch = GraphPatch::new(
            "test",
            vec![GraphPatchOp::AddNodeToGroup { node_id, group_id }],
        );
        g.apply_patch(&patch).unwrap();
        assert!(g.schema.groups[&group_id].node_ids.contains(&node_id));

        let patch2 = GraphPatch::new(
            "test",
            vec![GraphPatchOp::RemoveNodeFromGroup { node_id, group_id }],
        );
        g.apply_patch(&patch2).unwrap();
        assert!(!g.schema.groups[&group_id].node_ids.contains(&node_id));
    }

    // ── Node provenance ───────────────────────────────────────────────────

    #[test]
    fn node_provenance_records_actor() {
        let mut node = GraphNode::new("k", "k");
        node.provenance = NodeProvenance::for_actor("alice");
        assert_eq!(node.provenance.created_by, "alice");
        assert!(node.provenance.modified_by.is_none());

        node.provenance.touch("bob");
        assert_eq!(node.provenance.modified_by.as_deref(), Some("bob"));
    }
}
