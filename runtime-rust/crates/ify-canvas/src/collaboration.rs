//! # collaboration — Collaborative Cursors and Edit Conflict Resolution
//!
//! Optional collaborative editing support for the infinity canvas.
//! Provides cursor presence tracking, edit attribution, and a lightweight
//! last-write-wins (LWW) conflict resolution strategy for concurrent node
//! position and parameter edits.
//!
//! The collaboration layer is designed to be transport-agnostic; it models
//! the state machine and conflict rules, while the actual transport
//! (WebSocket, mesh bus, etc.) is injected by the caller.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::selection::Point;

// ---------------------------------------------------------------------------
// CollaboratorId
// ---------------------------------------------------------------------------

/// A unique identifier for a collaborator session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CollaboratorId(String);

impl CollaboratorId {
    /// Create a collaborator ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Return the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CollaboratorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// CursorPresence
// ---------------------------------------------------------------------------

/// Cursor position and metadata for a single collaborator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorPresence {
    /// Collaborator identifier.
    pub collaborator_id: CollaboratorId,
    /// Human-readable display name.
    pub display_name: String,
    /// Cursor position in canvas coordinates.
    pub position: Point,
    /// Hex colour string for the cursor (e.g. `"#FF5733"`).
    pub color: String,
    /// Milliseconds since epoch when this presence was last updated.
    pub updated_at_ms: u64,
    /// Node ID the collaborator has focused, if any.
    pub focused_node: Option<String>,
}

// ---------------------------------------------------------------------------
// PresenceStore
// ---------------------------------------------------------------------------

/// Tracks cursor presence for all active collaborators.
#[derive(Debug, Default, Clone)]
pub struct PresenceStore {
    cursors: HashMap<CollaboratorId, CursorPresence>,
}

impl PresenceStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update a collaborator's cursor.
    pub fn upsert(&mut self, presence: CursorPresence) {
        self.cursors.insert(presence.collaborator_id.clone(), presence);
    }

    /// Remove a collaborator (they disconnected).
    pub fn remove(&mut self, id: &CollaboratorId) {
        self.cursors.remove(id);
    }

    /// Return all current cursor presences.
    pub fn all(&self) -> impl Iterator<Item = &CursorPresence> {
        self.cursors.values()
    }

    /// Number of active collaborators.
    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    /// Returns `true` if no collaborators are tracked.
    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }
}

// ---------------------------------------------------------------------------
// EditOperation
// ---------------------------------------------------------------------------

/// A versioned canvas edit operation used for conflict resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditOperation {
    /// Globally unique operation ID.
    pub op_id: String,
    /// The collaborator who authored this operation.
    pub author: CollaboratorId,
    /// Milliseconds since epoch when this operation was generated.
    pub timestamp_ms: u64,
    /// The targeted node ID.
    pub node_id: String,
    /// The specific change being applied.
    pub change: NodeChange,
}

/// The change payload of an [`EditOperation`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeChange {
    /// Move a node to a new position.
    Move {
        /// New position.
        position: Point,
    },
    /// Update a node parameter.
    SetParameter {
        /// Parameter name.
        name: String,
        /// New serialised value.
        value: serde_json::Value,
    },
    /// Add an edge between two ports.
    AddEdge {
        /// Source node ID.
        from_node: String,
        /// Source port name.
        from_port: String,
        /// Target node ID.
        to_node: String,
        /// Target port name.
        to_port: String,
    },
    /// Remove an existing edge.
    RemoveEdge {
        /// Source node ID.
        from_node: String,
        /// Source port name.
        from_port: String,
        /// Target node ID.
        to_node: String,
        /// Target port name.
        to_port: String,
    },
}

// ---------------------------------------------------------------------------
// ConflictResolver
// ---------------------------------------------------------------------------

/// Last-write-wins (LWW) conflict resolver for concurrent edits.
///
/// When two operations target the same node field, the one with the higher
/// `timestamp_ms` wins.  Ties are broken lexicographically by `op_id`.
#[derive(Debug, Default)]
pub struct ConflictResolver {
    /// The most recently accepted operation per `(node_id, field_key)`.
    accepted: HashMap<String, EditOperation>,
}

impl ConflictResolver {
    /// Create a new resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to apply `op`.
    ///
    /// Returns `true` if the operation was accepted (wins the conflict
    /// check), `false` if it was rejected because a newer operation for
    /// the same target already exists.
    pub fn apply(&mut self, op: EditOperation) -> bool {
        let key = Self::field_key(&op);
        if let Some(existing) = self.accepted.get(&key) {
            let newer = op.timestamp_ms > existing.timestamp_ms
                || (op.timestamp_ms == existing.timestamp_ms && op.op_id > existing.op_id);
            if !newer {
                return false;
            }
        }
        self.accepted.insert(key, op);
        true
    }

    /// Return the accepted operation for a given field key, if any.
    pub fn accepted_op(&self, key: &str) -> Option<&EditOperation> {
        self.accepted.get(key)
    }

    fn field_key(op: &EditOperation) -> String {
        match &op.change {
            NodeChange::Move { .. } => format!("{}:position", op.node_id),
            NodeChange::SetParameter { name, .. } => format!("{}:param:{}", op.node_id, name),
            NodeChange::AddEdge { from_node, from_port, to_node, to_port } => {
                format!("edge:{}:{}->{}:{}", from_node, from_port, to_node, to_port)
            }
            NodeChange::RemoveEdge { from_node, from_port, to_node, to_port } => {
                format!("edge:{}:{}->{}:{}", from_node, from_port, to_node, to_port)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_op(op_id: &str, node_id: &str, ts: u64, x: f64) -> EditOperation {
        EditOperation {
            op_id: op_id.into(),
            author: CollaboratorId::new("alice"),
            timestamp_ms: ts,
            node_id: node_id.into(),
            change: NodeChange::Move {
                position: Point::new(x, 0.0),
            },
        }
    }

    #[test]
    fn presence_store_upsert_remove() {
        let mut store = PresenceStore::new();
        store.upsert(CursorPresence {
            collaborator_id: CollaboratorId::new("bob"),
            display_name: "Bob".into(),
            position: Point::new(10.0, 20.0),
            color: "#00FF00".into(),
            updated_at_ms: 1000,
            focused_node: None,
        });
        assert_eq!(store.len(), 1);
        store.remove(&CollaboratorId::new("bob"));
        assert!(store.is_empty());
    }

    #[test]
    fn conflict_resolver_newer_wins() {
        let mut resolver = ConflictResolver::new();
        let op_old = make_op("op-1", "n1", 1000, 10.0);
        let op_new = make_op("op-2", "n1", 2000, 20.0);

        assert!(resolver.apply(op_old));
        assert!(resolver.apply(op_new));

        let accepted = resolver.accepted_op("n1:position").unwrap();
        assert_eq!(accepted.op_id, "op-2");
    }

    #[test]
    fn conflict_resolver_older_rejected() {
        let mut resolver = ConflictResolver::new();
        let op_new = make_op("op-2", "n1", 2000, 20.0);
        let op_old = make_op("op-1", "n1", 1000, 10.0);

        resolver.apply(op_new);
        let accepted = resolver.apply(op_old);
        assert!(!accepted);
    }

    #[test]
    fn conflict_resolver_tie_broken_by_op_id() {
        let mut resolver = ConflictResolver::new();
        let op_a = make_op("op-a", "n1", 1000, 1.0);
        let op_z = make_op("op-z", "n1", 1000, 2.0);

        resolver.apply(op_a);
        resolver.apply(op_z.clone());

        let accepted = resolver.accepted_op("n1:position").unwrap();
        assert_eq!(accepted.op_id, "op-z");
    }

    #[test]
    fn node_change_parameter_key() {
        let op = EditOperation {
            op_id: "x".into(),
            author: CollaboratorId::new("alice"),
            timestamp_ms: 0,
            node_id: "n1".into(),
            change: NodeChange::SetParameter {
                name: "url".into(),
                value: serde_json::json!("https://example.com"),
            },
        };
        let key = ConflictResolver::field_key(&op);
        assert_eq!(key, "n1:param:url");
    }
}
