//! Node selection, data sampling, and inter-node communication — Epic F (node
//! communication and data sampling improvements).
//!
//! ## Overview
//!
//! ### Node selection (`NodeSelector` + `NodeSample`)
//!
//! Build a query with typed conditions, then apply it to a [`FlowGraphSchema`]
//! to get a [`NodeSample`].  The sample supports **count**, **range** (slice),
//! **scale** (proportional subset), **first / last**, **group by kind**, and
//! **sort by label**.
//!
//! ```rust,no_run
//! # use ify_controller::graph_query::{NodeSelector, NodeCondition};
//! # use ify_controller::graph::FlowGraphSchema;
//! # let schema = FlowGraphSchema::new(ify_core::DimensionId::new());
//! let sample = NodeSelector::new()
//!     .with_kind("http.request")
//!     .with_param_exists("url")
//!     .apply(&schema);
//!
//! println!("{} matching nodes", sample.count());
//! for node in sample.range(0, 5).iter() {
//!     println!("  {}", node.label);
//! }
//! ```
//!
//! ### Node communication (`NodeCommunicator`)
//!
//! Send messages between nodes through named channels.  Each node has an
//! in-process **inbox** that accumulates messages until they are drained.
//! Broadcast delivers a copy to every node registered in the graph.
//!
//! ```rust,no_run
//! # use ify_controller::graph_query::{NodeCommunicator, NodeMessage};
//! # use ify_controller::graph::{FlowGraph, GraphNode};
//! # use ify_controller::action_log::ActionLog;
//! # use ify_core::{DimensionId, TaskId};
//! # use std::sync::Arc;
//! # let log = ActionLog::new(16);
//! # let dim = DimensionId::new();
//! # let task = TaskId::new();
//! # let mut graph = FlowGraph::new(dim, task, Arc::clone(&log));
//! # let node_a = graph.add_node(GraphNode::new("a", "A"));
//! # let node_b = graph.add_node(GraphNode::new("b", "B"));
//! let mut comm = NodeCommunicator::new(dim, task, log);
//!
//! // Unicast: A → B
//! comm.send(NodeMessage::to(node_a, node_b, "data", serde_json::json!(42)));
//!
//! // Broadcast: A → all
//! comm.broadcast(node_a, "ping", serde_json::json!(null), graph.schema.nodes.keys().copied());
//!
//! // Drain B's inbox
//! let msgs = comm.drain_inbox(node_b);
//! assert_eq!(msgs.len(), 2);  // unicast + broadcast
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};
use crate::graph::{FlowGraphSchema, GraphNode, PortDataType, PortDirection};

// ---------------------------------------------------------------------------
// NodeCondition — composable, typed predicates
// ---------------------------------------------------------------------------

/// A composable, typed predicate over a [`GraphNode`].
///
/// Conditions are evaluated by [`NodeSelector::apply`] against each node in
/// the schema.  Compound conditions (AND / OR / NOT) allow arbitrary boolean
/// logic.
#[derive(Debug, Clone)]
pub enum NodeCondition {
    /// Node `kind` equals the given string (exact match).
    KindEq(String),
    /// Node `label` contains the given substring (case-sensitive).
    LabelContains(String),
    /// Node `label` equals the given string (exact match).
    LabelEq(String),
    /// Node parameters contain the given key with the given JSON value.
    ParamEq {
        /// Parameter key.
        key: String,
        /// Expected value.
        value: serde_json::Value,
    },
    /// Node parameters contain the given key (any value).
    ParamExists(String),
    /// Node belongs to the group with the given ID.
    InGroup(Uuid),
    /// Node has at least one input port.
    HasInputPort,
    /// Node has at least one output port.
    HasOutputPort,
    /// Node has at least one port with the given data type (any direction).
    HasPortOfType(PortDataType),
    /// Node has at least one port with the given data type *and* direction.
    HasPortMatching {
        /// Required port direction.
        direction: PortDirection,
        /// Required port data type.
        data_type: PortDataType,
    },
    /// Node is a subgraph reference.
    IsSubgraphRef,
    /// Negation of the inner condition.
    Not(Box<NodeCondition>),
    /// All inner conditions must hold.
    And(Vec<NodeCondition>),
    /// At least one inner condition must hold.
    Or(Vec<NodeCondition>),
}

impl NodeCondition {
    /// Evaluate this condition against a node and the surrounding schema.
    fn matches(&self, node: &GraphNode, schema: &FlowGraphSchema) -> bool {
        match self {
            Self::KindEq(k) => node.kind == *k,
            Self::LabelContains(s) => node.label.contains(s.as_str()),
            Self::LabelEq(s) => node.label == *s,
            Self::ParamEq { key, value } => {
                node.parameters.get(key.as_str()) == Some(value)
            }
            Self::ParamExists(key) => node.parameters.contains_key(key.as_str()),
            Self::InGroup(gid) => schema
                .groups
                .get(gid)
                .map(|g| g.node_ids.contains(&node.id))
                .unwrap_or(false),
            Self::HasInputPort => node.ports.values().any(|p| p.direction == PortDirection::In),
            Self::HasOutputPort => node.ports.values().any(|p| p.direction == PortDirection::Out),
            Self::HasPortOfType(dt) => node.ports.values().any(|p| p.data_type == *dt),
            Self::HasPortMatching { direction, data_type } => node
                .ports
                .values()
                .any(|p| p.direction == *direction && p.data_type == *data_type),
            Self::IsSubgraphRef => node.is_subgraph_ref(),
            Self::Not(inner) => !inner.matches(node, schema),
            Self::And(conds) => conds.iter().all(|c| c.matches(node, schema)),
            Self::Or(conds) => conds.iter().any(|c| c.matches(node, schema)),
        }
    }
}

// ---------------------------------------------------------------------------
// NodeSelector — fluent query builder
// ---------------------------------------------------------------------------

/// Fluent builder for selecting a subset of nodes from a [`FlowGraphSchema`].
///
/// All added conditions are combined with AND semantics.  Use
/// [`NodeCondition::Or`] / [`NodeCondition::And`] explicitly to build more
/// complex expressions and attach them with [`Self::with_condition`].
///
/// # Example
///
/// ```rust,no_run
/// # use ify_controller::graph_query::NodeSelector;
/// # use ify_controller::graph::FlowGraphSchema;
/// # let schema = FlowGraphSchema::new(ify_core::DimensionId::new());
/// let sample = NodeSelector::new()
///     .with_kind("ml.predict")
///     .with_param_exists("model_id")
///     .apply(&schema);
/// println!("{} nodes matched", sample.count());
/// ```
#[derive(Debug, Default)]
pub struct NodeSelector {
    conditions: Vec<NodeCondition>,
}

impl NodeSelector {
    /// Create a selector that will match all nodes (no conditions yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter to nodes whose `kind` exactly matches `kind`.
    pub fn with_kind(mut self, kind: impl Into<String>) -> Self {
        self.conditions.push(NodeCondition::KindEq(kind.into()));
        self
    }

    /// Filter to nodes whose `label` contains `substr`.
    pub fn with_label_containing(mut self, substr: impl Into<String>) -> Self {
        self.conditions.push(NodeCondition::LabelContains(substr.into()));
        self
    }

    /// Filter to nodes whose `label` exactly equals `label`.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.conditions.push(NodeCondition::LabelEq(label.into()));
        self
    }

    /// Filter to nodes that have parameter `key` set to `value`.
    pub fn with_param_eq(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.conditions.push(NodeCondition::ParamEq { key: key.into(), value });
        self
    }

    /// Filter to nodes that have parameter `key` set to any value.
    pub fn with_param_exists(mut self, key: impl Into<String>) -> Self {
        self.conditions.push(NodeCondition::ParamExists(key.into()));
        self
    }

    /// Filter to nodes that belong to `group_id`.
    pub fn in_group(mut self, group_id: Uuid) -> Self {
        self.conditions.push(NodeCondition::InGroup(group_id));
        self
    }

    /// Filter to nodes that have at least one input port.
    pub fn having_input_port(mut self) -> Self {
        self.conditions.push(NodeCondition::HasInputPort);
        self
    }

    /// Filter to nodes that have at least one output port.
    pub fn having_output_port(mut self) -> Self {
        self.conditions.push(NodeCondition::HasOutputPort);
        self
    }

    /// Filter to nodes that have at least one port of the given type.
    pub fn having_port_of_type(mut self, data_type: PortDataType) -> Self {
        self.conditions.push(NodeCondition::HasPortOfType(data_type));
        self
    }

    /// Filter to nodes that have at least one port matching both direction and
    /// data type.
    pub fn having_port_matching(
        mut self,
        direction: PortDirection,
        data_type: PortDataType,
    ) -> Self {
        self.conditions
            .push(NodeCondition::HasPortMatching { direction, data_type });
        self
    }

    /// Filter to subgraph-reference nodes only.
    pub fn subgraph_refs_only(mut self) -> Self {
        self.conditions.push(NodeCondition::IsSubgraphRef);
        self
    }

    /// Attach an arbitrary [`NodeCondition`] (enabling full boolean logic).
    pub fn with_condition(mut self, cond: NodeCondition) -> Self {
        self.conditions.push(cond);
        self
    }

    /// Execute the query against `schema` and return a [`NodeSample`].
    ///
    /// Nodes in the result are in `BTreeMap` iteration order (sorted by ID),
    /// giving a deterministic result across calls.
    pub fn apply<'s>(&self, schema: &'s FlowGraphSchema) -> NodeSample<'s> {
        let nodes: Vec<&'s GraphNode> = schema
            .nodes
            .values()
            .filter(|n| self.conditions.iter().all(|c| c.matches(n, schema)))
            .collect();
        NodeSample { nodes }
    }
}

// ---------------------------------------------------------------------------
// NodeSample — aggregation and sampling over a result set
// ---------------------------------------------------------------------------

/// An immutable, ordered result set of node references produced by
/// [`NodeSelector::apply`].
///
/// Provides aggregation primitives — count, range slice, proportional scale,
/// first/last, group-by-kind — for data sampling and batch operations.
pub struct NodeSample<'s> {
    nodes: Vec<&'s GraphNode>,
}

impl<'s> NodeSample<'s> {
    /// Wrap a pre-computed list of node references into a sample.
    pub fn from_nodes(nodes: Vec<&'s GraphNode>) -> Self {
        Self { nodes }
    }

    /// Number of nodes in this sample.
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// `true` if the sample contains no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Return a sub-sample covering `[start, end)` indices.
    ///
    /// Indices are clamped to the actual sample length so out-of-bounds
    /// values never panic.
    pub fn range(&self, start: usize, end: usize) -> NodeSample<'_> {
        let start = start.min(self.nodes.len());
        let end = end.min(self.nodes.len());
        NodeSample { nodes: self.nodes[start..end].to_vec() }
    }

    /// Return a proportionally-sized subset of this sample.
    ///
    /// `factor` is clamped to `[0.0, 1.0]`.  The returned sample contains
    /// `floor(count * factor)` nodes, evenly distributed across the original
    /// ordering.  If the result would be empty but the original was not, at
    /// least one node is returned.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # let sample: ify_controller::graph_query::NodeSample<'_> =
    /// #     ify_controller::graph_query::NodeSample::from_nodes(vec![]);
    /// let half = sample.scale(0.5);  // ~50 % of nodes
    /// let quarter = sample.scale(0.25);  // ~25 % of nodes
    /// ```
    pub fn scale(&self, factor: f64) -> NodeSample<'_> {
        if self.nodes.is_empty() {
            return NodeSample { nodes: vec![] };
        }
        let factor = factor.clamp(0.0, 1.0);
        let target = ((self.nodes.len() as f64) * factor).floor() as usize;
        // Guarantee at least 1 if the sample is non-empty and factor > 0.
        let target = if target == 0 && factor > 0.0 { 1 } else { target };

        if target >= self.nodes.len() {
            return NodeSample { nodes: self.nodes.clone() };
        }

        // Evenly-spaced stride selection.
        let stride = self.nodes.len() as f64 / target as f64;
        let selected: Vec<&'s GraphNode> = (0..target)
            .map(|i| {
                let idx = (i as f64 * stride).floor() as usize;
                self.nodes[idx.min(self.nodes.len() - 1)]
            })
            .collect();
        NodeSample { nodes: selected }
    }

    /// Return the first node in the sample, or `None` if empty.
    pub fn first(&self) -> Option<&'s GraphNode> {
        self.nodes.first().copied()
    }

    /// Return the last node in the sample, or `None` if empty.
    pub fn last(&self) -> Option<&'s GraphNode> {
        self.nodes.last().copied()
    }

    /// Collect all node IDs in the sample.
    pub fn ids(&self) -> Vec<Uuid> {
        self.nodes.iter().map(|n| n.id).collect()
    }

    /// Iterate over node references.
    pub fn iter(&self) -> std::slice::Iter<'_, &'s GraphNode> {
        self.nodes.iter()
    }

    /// Group nodes by their `kind` string.
    ///
    /// Returns a `HashMap<kind, Vec<&GraphNode>>` for bucketed processing.
    pub fn group_by_kind(&self) -> HashMap<&str, Vec<&'s GraphNode>> {
        let mut map: HashMap<&str, Vec<&'s GraphNode>> = HashMap::new();
        for &node in &self.nodes {
            map.entry(node.kind.as_str()).or_default().push(node);
        }
        map
    }

    /// Return a new sample with nodes sorted by label (ascending).
    pub fn sort_by_label(mut self) -> Self {
        self.nodes.sort_by(|a, b| a.label.cmp(&b.label));
        self
    }

    /// Return a new sample with nodes sorted by ID (ascending — deterministic).
    pub fn sort_by_id(mut self) -> Self {
        self.nodes.sort_by_key(|n| n.id);
        self
    }

    /// Consume the sample and return the underlying `Vec<&GraphNode>`.
    pub fn into_vec(self) -> Vec<&'s GraphNode> {
        self.nodes
    }
}

// ---------------------------------------------------------------------------
// NodeMessage — inter-node communication payload
// ---------------------------------------------------------------------------

/// A message sent from one node to another (or broadcast to all).
///
/// Messages are buffered in the [`NodeCommunicator`] inbox until the
/// recipient drains them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMessage {
    /// Unique message identifier.
    pub id: Uuid,
    /// Node that sent the message.
    pub from_node: Uuid,
    /// Destination node, or `None` for a broadcast.
    pub to_node: Option<Uuid>,
    /// Named channel — consumers filter by channel name.
    pub channel: String,
    /// Arbitrary message payload.
    pub payload: serde_json::Value,
    /// Unix epoch milliseconds when the message was sent.
    pub sent_at_ms: u64,
}

impl NodeMessage {
    /// Construct a unicast message from `from_node` to `to_node` on `channel`.
    pub fn to(
        from_node: Uuid,
        to_node: Uuid,
        channel: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            from_node,
            to_node: Some(to_node),
            channel: channel.into(),
            payload,
            sent_at_ms: now_ms(),
        }
    }

    /// Construct a broadcast message (no specific recipient).
    pub fn broadcast(
        from_node: Uuid,
        channel: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            from_node,
            to_node: None,
            channel: channel.into(),
            payload,
            sent_at_ms: now_ms(),
        }
    }
}

// ---------------------------------------------------------------------------
// NodeCommunicator — in-process message bus
// ---------------------------------------------------------------------------

/// In-process message bus for inter-node communication.
///
/// Each node has an **inbox** — a `Vec<NodeMessage>` that accumulates incoming
/// messages until the consumer calls [`Self::drain_inbox`].
///
/// # Unicast vs broadcast
///
/// * [`Self::send`] — deliver to a single recipient node.
/// * [`Self::broadcast`] — deliver a copy to every node ID supplied as an
///   iterator; typically all nodes in the graph.
///
/// All operations emit [`ActionLogEntry`] events for auditability.
pub struct NodeCommunicator {
    inbox: HashMap<Uuid, Vec<NodeMessage>>,
    action_log: Arc<ActionLog>,
    dimension_id: DimensionId,
    task_id: TaskId,
}

impl NodeCommunicator {
    /// Create a new communicator.
    pub fn new(
        dimension_id: DimensionId,
        task_id: TaskId,
        action_log: Arc<ActionLog>,
    ) -> Self {
        Self {
            inbox: HashMap::new(),
            action_log,
            dimension_id,
            task_id,
        }
    }

    /// Send a unicast [`NodeMessage`] to its `to_node` recipient.
    ///
    /// If the message has no `to_node` set it is silently dropped (use
    /// [`Self::broadcast`] instead).
    ///
    /// Emits a [`EventType::NodeMessageSent`] event.
    pub fn send(&mut self, msg: NodeMessage) {
        let Some(to) = msg.to_node else { return };
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeMessageSent,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "msg_id": msg.id,
                "from": msg.from_node,
                "to": to,
                "channel": msg.channel,
            }),
        ));
        self.inbox.entry(to).or_default().push(msg);
    }

    /// Broadcast a message to every node in `recipients`.
    ///
    /// A separate [`NodeMessage`] copy (with a new ID) is delivered to each
    /// recipient.  Emits a single [`EventType::NodeMessageBroadcast`] event.
    ///
    /// Returns the number of recipients that received the message.
    pub fn broadcast(
        &mut self,
        from_node: Uuid,
        channel: impl Into<String>,
        payload: serde_json::Value,
        recipients: impl IntoIterator<Item = Uuid>,
    ) -> usize {
        let channel = channel.into();
        let recipients: Vec<Uuid> = recipients.into_iter().collect();
        let count = recipients.len();
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeMessageBroadcast,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "from": from_node,
                "channel": channel,
                "recipient_count": count,
            }),
        ));
        for to in recipients {
            let msg = NodeMessage {
                id: Uuid::now_v7(),
                from_node,
                to_node: Some(to),
                channel: channel.clone(),
                payload: payload.clone(),
                sent_at_ms: now_ms(),
            };
            self.inbox.entry(to).or_default().push(msg);
        }
        count
    }

    /// Return a slice of all pending messages in `node_id`'s inbox.
    ///
    /// Messages remain in the inbox until [`Self::drain_inbox`] is called.
    pub fn inbox_for(&self, node_id: Uuid) -> &[NodeMessage] {
        self.inbox.get(&node_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Return pending messages on a specific `channel` in `node_id`'s inbox.
    pub fn inbox_for_channel<'a>(
        &'a self,
        node_id: Uuid,
        channel: &str,
    ) -> Vec<&'a NodeMessage> {
        self.inbox_for(node_id)
            .iter()
            .filter(|m| m.channel == channel)
            .collect()
    }

    /// Remove and return all messages from `node_id`'s inbox.
    pub fn drain_inbox(&mut self, node_id: Uuid) -> Vec<NodeMessage> {
        self.inbox.remove(&node_id).unwrap_or_default()
    }

    /// Remove and return only messages on `channel` from `node_id`'s inbox;
    /// messages on other channels remain.
    pub fn drain_channel(&mut self, node_id: Uuid, channel: &str) -> Vec<NodeMessage> {
        let inbox = self.inbox.entry(node_id).or_default();
        let (drained, remaining): (Vec<_>, Vec<_>) =
            inbox.drain(..).partition(|m| m.channel == channel);
        *inbox = remaining;
        drained
    }

    /// Number of pending messages in `node_id`'s inbox.
    pub fn pending_count(&self, node_id: Uuid) -> usize {
        self.inbox_for(node_id).len()
    }

    /// Total number of pending messages across all node inboxes.
    pub fn total_pending(&self) -> usize {
        self.inbox.values().map(Vec::len).sum()
    }

    /// `true` if the given node has no pending messages.
    pub fn inbox_is_empty(&self, node_id: Uuid) -> bool {
        self.inbox_for(node_id).is_empty()
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
    use crate::graph::{
        FlowGraph, FlowGraphSchema, GraphNode, Group, Link, NodeRelation, PortDef, RelationKind,
        GRAPH_SCHEMA_VERSION,
    };

    // ── Fixtures ─────────────────────────────────────────────────────────

    fn make_log() -> Arc<ActionLog> {
        ActionLog::new(64)
    }

    fn make_schema_with_nodes() -> FlowGraphSchema {
        let dim = DimensionId::new();
        let mut schema = FlowGraphSchema::new(dim);

        let kinds = [
            ("http.request", "Fetch A"),
            ("http.request", "Fetch B"),
            ("ml.predict", "Predict"),
            ("db.query", "Query DB"),
            ("db.query", "Archive DB"),
        ];
        for (kind, label) in kinds {
            let mut node = GraphNode::new(kind, label);
            node.parameters
                .insert("url".into(), serde_json::json!("https://example.com"));
            schema.nodes.insert(node.id, node);
        }
        schema
    }

    fn make_graph() -> FlowGraph {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        FlowGraph::new(dim, task, log)
    }

    // ── NodeSelector ──────────────────────────────────────────────────────

    #[test]
    fn selector_no_conditions_returns_all() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        assert_eq!(sample.count(), schema.nodes.len());
    }

    #[test]
    fn selector_by_kind() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().with_kind("http.request").apply(&schema);
        assert_eq!(sample.count(), 2);
        assert!(sample.iter().all(|n| n.kind == "http.request"));
    }

    #[test]
    fn selector_by_kind_no_match() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().with_kind("trading.order").apply(&schema);
        assert!(sample.is_empty());
    }

    #[test]
    fn selector_by_label_containing() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().with_label_containing("DB").apply(&schema);
        assert_eq!(sample.count(), 2);
    }

    #[test]
    fn selector_by_label_exact() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().with_label("Predict").apply(&schema);
        assert_eq!(sample.count(), 1);
        assert_eq!(sample.first().unwrap().label, "Predict");
    }

    #[test]
    fn selector_param_exists() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().with_param_exists("url").apply(&schema);
        // all 5 nodes have "url"
        assert_eq!(sample.count(), 5);
    }

    #[test]
    fn selector_param_eq() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new()
            .with_param_eq("url", serde_json::json!("https://example.com"))
            .apply(&schema);
        assert_eq!(sample.count(), 5);
    }

    #[test]
    fn selector_in_group() {
        let mut schema = make_schema_with_nodes();
        let mut group = Group::new("G");
        let ids: Vec<Uuid> = schema.nodes.keys().take(2).copied().collect();
        group.node_ids.extend(&ids);
        schema.groups.insert(group.id, group.clone());

        let sample = NodeSelector::new().in_group(group.id).apply(&schema);
        assert_eq!(sample.count(), 2);
    }

    #[test]
    fn selector_has_port_matching() {
        let mut schema = make_schema_with_nodes();
        // Add a port to one node.
        let id = *schema.nodes.keys().next().unwrap();
        let port = PortDef::new("out", PortDirection::Out, PortDataType::Number);
        schema.nodes.get_mut(&id).unwrap().add_port(port).unwrap();

        let sample = NodeSelector::new()
            .having_port_matching(PortDirection::Out, PortDataType::Number)
            .apply(&schema);
        assert_eq!(sample.count(), 1);
    }

    #[test]
    fn selector_not_condition() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new()
            .with_condition(NodeCondition::Not(Box::new(NodeCondition::KindEq(
                "http.request".into(),
            ))))
            .apply(&schema);
        assert_eq!(sample.count(), 3); // ml.predict + 2 x db.query
    }

    #[test]
    fn selector_or_condition() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new()
            .with_condition(NodeCondition::Or(vec![
                NodeCondition::KindEq("ml.predict".into()),
                NodeCondition::KindEq("db.query".into()),
            ]))
            .apply(&schema);
        assert_eq!(sample.count(), 3);
    }

    #[test]
    fn selector_and_stacks_conditions() {
        let schema = make_schema_with_nodes();
        // kind == "db.query" AND label contains "Archive"
        let sample = NodeSelector::new()
            .with_kind("db.query")
            .with_label_containing("Archive")
            .apply(&schema);
        assert_eq!(sample.count(), 1);
        assert_eq!(sample.first().unwrap().label, "Archive DB");
    }

    // ── NodeSample aggregation ────────────────────────────────────────────

    #[test]
    fn sample_range_slices_correctly() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        let sliced = sample.range(1, 3);
        assert_eq!(sliced.count(), 2);
    }

    #[test]
    fn sample_range_out_of_bounds_clamps() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        let all = sample.range(0, 9999);
        assert_eq!(all.count(), 5);
    }

    #[test]
    fn sample_scale_half() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        let half = sample.scale(0.5);
        assert_eq!(half.count(), 2); // floor(5 * 0.5) = 2
    }

    #[test]
    fn sample_scale_zero_returns_empty() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        let empty = sample.scale(0.0);
        assert!(empty.is_empty());
    }

    #[test]
    fn sample_scale_full() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        let full = sample.scale(1.0);
        assert_eq!(full.count(), 5);
    }

    #[test]
    fn sample_scale_clamps_above_one() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        let clamped = sample.scale(2.0);
        assert_eq!(clamped.count(), 5);
    }

    #[test]
    fn sample_first_and_last() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().with_kind("http.request").apply(&schema);
        assert!(sample.first().is_some());
        assert!(sample.last().is_some());
    }

    #[test]
    fn sample_group_by_kind() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema);
        let grouped = sample.group_by_kind();
        assert_eq!(grouped["http.request"].len(), 2);
        assert_eq!(grouped["db.query"].len(), 2);
        assert_eq!(grouped["ml.predict"].len(), 1);
    }

    #[test]
    fn sample_sort_by_label() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().apply(&schema).sort_by_label();
        let labels: Vec<&str> = sample.iter().map(|n| n.label.as_str()).collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        assert_eq!(labels, sorted);
    }

    #[test]
    fn sample_ids() {
        let schema = make_schema_with_nodes();
        let sample = NodeSelector::new().with_kind("db.query").apply(&schema);
        let ids = sample.ids();
        assert_eq!(ids.len(), 2);
        for id in &ids {
            assert_eq!(schema.nodes[id].kind, "db.query");
        }
    }

    // ── NodeRelation operations ───────────────────────────────────────────

    #[test]
    fn add_and_query_relation() {
        let mut g = make_graph();
        let a = g.add_node(GraphNode::new("src", "A"));
        let b = g.add_node(GraphNode::new("tgt", "B"));

        let rel = NodeRelation::new(a, b, RelationKind::Triggers);
        let rel_id = g.add_relation(rel).unwrap();

        let rels = g.relations_for_node(a);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].id, rel_id);
        assert_eq!(rels[0].kind, RelationKind::Triggers);
    }

    #[test]
    fn relation_missing_node_fails() {
        let mut g = make_graph();
        let a = g.add_node(GraphNode::new("src", "A"));
        let missing = Uuid::new_v4();

        let rel = NodeRelation::new(a, missing, RelationKind::DependsOn);
        assert!(matches!(
            g.add_relation(rel),
            Err(crate::graph::FlowGraphError::NodeNotFound(_))
        ));
    }

    #[test]
    fn remove_relation() {
        let mut g = make_graph();
        let a = g.add_node(GraphNode::new("A", "A"));
        let b = g.add_node(GraphNode::new("B", "B"));

        let rel = NodeRelation::new(a, b, RelationKind::ObservedBy);
        let rid = g.add_relation(rel).unwrap();
        g.remove_relation(rid).unwrap();

        assert!(g.relations_for_node(a).is_empty());
    }

    #[test]
    fn outgoing_and_incoming_relations() {
        let mut g = make_graph();
        let a = g.add_node(GraphNode::new("A", "A"));
        let b = g.add_node(GraphNode::new("B", "B"));

        let rel = NodeRelation::new(a, b, RelationKind::ProvidesDataTo);
        g.add_relation(rel).unwrap();

        assert_eq!(g.outgoing_relations(a).len(), 1);
        assert_eq!(g.incoming_relations(b).len(), 1);
        assert!(g.outgoing_relations(b).is_empty());
        assert!(g.incoming_relations(a).is_empty());
    }

    #[test]
    fn relation_kind_custom() {
        let kind = RelationKind::Custom("co_located".into());
        assert_eq!(kind.as_str(), "co_located");
    }

    // ── NodeCommunicator ─────────────────────────────────────────────────

    #[test]
    fn send_unicast_message() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mut comm = NodeCommunicator::new(dim, task, log);

        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        comm.send(NodeMessage::to(from, to, "data", serde_json::json!(42)));

        assert_eq!(comm.pending_count(to), 1);
        assert_eq!(comm.inbox_for(to)[0].payload, serde_json::json!(42));
    }

    #[test]
    fn broadcast_delivers_to_all() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mut comm = NodeCommunicator::new(dim, task, log);

        let from = Uuid::new_v4();
        let recipients: Vec<Uuid> = (0..4).map(|_| Uuid::new_v4()).collect();

        let count = comm.broadcast(
            from,
            "ping",
            serde_json::json!(null),
            recipients.iter().copied(),
        );
        assert_eq!(count, 4);
        for id in &recipients {
            assert_eq!(comm.pending_count(*id), 1);
        }
        assert_eq!(comm.total_pending(), 4);
    }

    #[test]
    fn drain_inbox_removes_messages() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mut comm = NodeCommunicator::new(dim, task, log);

        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        comm.send(NodeMessage::to(from, to, "ch", serde_json::json!(1)));
        comm.send(NodeMessage::to(from, to, "ch", serde_json::json!(2)));

        let drained = comm.drain_inbox(to);
        assert_eq!(drained.len(), 2);
        assert!(comm.inbox_is_empty(to));
    }

    #[test]
    fn drain_channel_leaves_other_channels() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mut comm = NodeCommunicator::new(dim, task, log);

        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        comm.send(NodeMessage::to(from, to, "alpha", serde_json::json!(1)));
        comm.send(NodeMessage::to(from, to, "beta", serde_json::json!(2)));

        let drained = comm.drain_channel(to, "alpha");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].channel, "alpha");
        // "beta" still present
        assert_eq!(comm.pending_count(to), 1);
    }

    #[test]
    fn inbox_for_channel_filters_correctly() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mut comm = NodeCommunicator::new(dim, task, log);

        let from = Uuid::new_v4();
        let to = Uuid::new_v4();
        comm.send(NodeMessage::to(from, to, "signals", serde_json::json!(1)));
        comm.send(NodeMessage::to(from, to, "signals", serde_json::json!(2)));
        comm.send(NodeMessage::to(from, to, "control", serde_json::json!(3)));

        let signals = comm.inbox_for_channel(to, "signals");
        assert_eq!(signals.len(), 2);
    }

    #[test]
    fn send_broadcast_message_is_null_safe() {
        // NodeMessage::broadcast with to_node == None is silently dropped by send()
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mut comm = NodeCommunicator::new(dim, task, log);
        let bcast = NodeMessage::broadcast(Uuid::new_v4(), "ch", serde_json::json!(0));
        comm.send(bcast); // should not panic, silently no-ops
        assert_eq!(comm.total_pending(), 0);
    }

    // ── Serialisation: relations field is backward-compatible ─────────────

    #[test]
    fn schema_without_relations_field_deserialises_ok() {
        // Old JSON without the "relations" key should still parse (default = empty).
        let dim = DimensionId::new();
        let json = serde_json::json!({
            "schema_version": GRAPH_SCHEMA_VERSION,
            "graph_id": Uuid::now_v7().to_string(),
            "dimension_id": dim.as_uuid().to_string(),
            "nodes": {},
            "links": {},
            "groups": {},
            "subgraphs": {}
            // no "relations" key
        })
        .to_string();
        let schema = FlowGraphSchema::from_json(&json).unwrap();
        assert!(schema.relations.is_empty());
    }

    // ── Scale determinism (data sampling harness) ────────────────────────

    #[test]
    fn scale_result_is_deterministic() {
        let schema = make_schema_with_nodes();
        let sample_a = NodeSelector::new().apply(&schema);
        let sample_b = NodeSelector::new().apply(&schema);
        let scaled1 = sample_a.scale(0.4);
        let scaled2 = sample_b.scale(0.4);
        assert_eq!(scaled1.ids(), scaled2.ids(), "scale must be deterministic");
    }
}
