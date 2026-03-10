//! Mesh-artifact write path: produce, consume, snapshot, diff/patch.
//!
//! Satisfies Epic B requirement:
//! > Implement mesh-artifact write path (produce/consume artifacts, node
//! > snapshots, diff patches).
//!
//! ## Artifact lifecycle
//!
//! ```text
//! produce() ──► COMMITTED ──► (consume | snapshot | patch)
//! ```
//!
//! Each operation emits an [`ActionLogEntry`] and broadcasts the
//! [`ArtifactId`] to subscribers.
//!
//! ## Alignment with spec
//!
//! The [`MeshArtifact`] provenance record mirrors the schema defined in
//! `docs/architecture/artifact-model.md §5`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ify_core::{ArtifactId, DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

use crate::action_log::{now_ms, ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by mesh artifact operations.
#[derive(Debug, Error)]
pub enum MeshError {
    /// The artifact was not found in the store.
    #[error("artifact {0} not found in mesh store")]
    NotFound(ArtifactId),

    /// An attempt was made to consume the same artifact twice.
    #[error("artifact {0} has already been consumed")]
    AlreadyConsumed(ArtifactId),
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// Provenance record linking an artifact to its producing execution chain.
///
/// Mirrors `docs/architecture/artifact-model.md §5`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactProvenance {
    /// Task that produced this artifact.
    pub producing_task_id: TaskId,
    /// Agent that produced this artifact, if any.
    pub producing_agent_id: Option<String>,
    /// Canvas node that produced this artifact, if any.
    pub producing_node_id: Option<Uuid>,
    /// Block controller that produced this artifact, if any.
    pub controller_id: Option<Uuid>,
    /// Schema version of the artifact content.
    pub schema_version: String,
}

// ---------------------------------------------------------------------------
// MeshArtifact
// ---------------------------------------------------------------------------

/// An immutable artifact stored in the mesh.
///
/// Immutability tier defaults to `1` (session-scoped).  Set `immutability_tier`
/// to `2` for persistent artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshArtifact {
    /// Unique, time-ordered artifact identifier.
    pub id: ArtifactId,
    /// Dimension this artifact belongs to.
    pub dimension_id: DimensionId,
    /// Task that produced this artifact.
    pub task_id: TaskId,
    /// Canvas node this artifact belongs to, if any.
    pub node_id: Option<Uuid>,
    /// MIME type or semantic content descriptor.
    pub content_type: String,
    /// Artifact payload.
    pub payload: serde_json::Value,
    /// Provenance chain.
    pub provenance: ArtifactProvenance,
    /// Unix epoch milliseconds when this artifact was produced.
    pub created_at_ms: u64,
    /// Immutability tier: 0=ephemeral, 1=session, 2=persistent.
    pub immutability_tier: u8,
    /// Whether this artifact has been consumed.
    pub consumed: bool,
}

// ---------------------------------------------------------------------------
// NodeSnapshot
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of a canvas node's state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSnapshot {
    /// The artifact ID for this snapshot.
    pub artifact_id: ArtifactId,
    /// The node this snapshot was taken from.
    pub node_id: Uuid,
    /// Serialized node state at capture time.
    pub state: serde_json::Value,
    /// Unix epoch milliseconds of capture.
    pub captured_at_ms: u64,
}

// ---------------------------------------------------------------------------
// DiffPatch
// ---------------------------------------------------------------------------

/// A diff/patch record capturing the delta between two artifact states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffPatch {
    /// The artifact ID for this patch record.
    pub artifact_id: ArtifactId,
    /// State before the patch.
    pub before: serde_json::Value,
    /// State after the patch.
    pub after: serde_json::Value,
    /// Ordered list of patch operations.
    pub ops: Vec<PatchOp>,
}

/// A single patch operation (JSON-Patch-inspired).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOp {
    /// Add a new value at `path`.
    Add {
        /// JSON Pointer path.
        path: String,
        /// Value to insert.
        value: serde_json::Value,
    },
    /// Remove the value at `path`.
    Remove {
        /// JSON Pointer path.
        path: String,
    },
    /// Replace an existing value at `path`.
    Replace {
        /// JSON Pointer path.
        path: String,
        /// Previous value (for audit).
        old: serde_json::Value,
        /// New value.
        new: serde_json::Value,
    },
}

// ---------------------------------------------------------------------------
// MeshArtifactStore
// ---------------------------------------------------------------------------

/// In-process mesh artifact store supporting produce, consume, snapshot, and
/// diff/patch operations.
///
/// All writes emit an [`ActionLogEntry`] and broadcast the new [`ArtifactId`]
/// to subscribers.
pub struct MeshArtifactStore {
    artifacts: Mutex<HashMap<ArtifactId, MeshArtifact>>,
    snapshots: Mutex<HashMap<ArtifactId, NodeSnapshot>>,
    patches: Mutex<HashMap<ArtifactId, DiffPatch>>,
    action_log: Arc<ActionLog>,
    tx: broadcast::Sender<ArtifactId>,
}

impl std::fmt::Debug for MeshArtifactStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.artifacts.lock().map(|g| g.len()).unwrap_or(0);
        write!(f, "MeshArtifactStore {{ artifacts: {count} }}")
    }
}

impl MeshArtifactStore {
    /// Create a new, empty mesh artifact store.
    ///
    /// `channel_capacity` controls the broadcast ring-buffer size.
    pub fn new(action_log: Arc<ActionLog>, channel_capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(channel_capacity.max(1));
        Arc::new(Self {
            artifacts: Mutex::new(HashMap::new()),
            snapshots: Mutex::new(HashMap::new()),
            patches: Mutex::new(HashMap::new()),
            action_log,
            tx,
        })
    }

    // ------------------------------------------------------------------
    // Produce
    // ------------------------------------------------------------------

    /// Produce an artifact and commit it to the store.
    ///
    /// Returns the assigned [`ArtifactId`].
    #[instrument(skip(self, artifact), fields(dimension = %artifact.dimension_id, task_id = %artifact.task_id))]
    pub fn produce(&self, mut artifact: MeshArtifact) -> ArtifactId {
        let id = artifact.id;
        artifact.created_at_ms = now_ms();
        artifact.consumed = false;

        let dimension_id = artifact.dimension_id;
        let task_id = artifact.task_id;

        {
            let mut arts = self.artifacts.lock().expect("mesh lock poisoned");
            arts.insert(id, artifact);
        }

        info!(artifact_id = %id, "mesh artifact produced");
        self.action_log.append(ActionLogEntry::new(
            EventType::ArtifactProduced,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "artifact_id": id.to_string(),
                "dimension_id": dimension_id.to_string(),
            }),
        ));

        self.notify(id);
        id
    }

    // ------------------------------------------------------------------
    // Consume
    // ------------------------------------------------------------------

    /// Consume (read) an artifact from the store.
    ///
    /// Consuming marks the artifact so callers can track reads without
    /// destroying the artifact (artifacts are immutable).
    ///
    /// # Errors
    ///
    /// - [`MeshError::NotFound`] if the ID is unknown.
    /// - [`MeshError::AlreadyConsumed`] if already consumed.
    #[instrument(skip(self), fields(artifact_id = %id))]
    pub fn consume(&self, id: ArtifactId) -> Result<MeshArtifact, MeshError> {
        let mut arts = self.artifacts.lock().expect("mesh lock poisoned");
        let artifact = arts.get_mut(&id).ok_or(MeshError::NotFound(id))?;

        if artifact.consumed {
            return Err(MeshError::AlreadyConsumed(id));
        }

        artifact.consumed = true;
        let result = artifact.clone();

        debug!(artifact_id = %id, "mesh artifact consumed");
        self.action_log.append(ActionLogEntry::new(
            EventType::ArtifactConsumed,
            Actor::System,
            Some(result.dimension_id),
            Some(result.task_id),
            serde_json::json!({
                "artifact_id": id.to_string(),
                "consumer_task_id": result.task_id.to_string(),
            }),
        ));

        Ok(result)
    }

    // ------------------------------------------------------------------
    // Snapshot
    // ------------------------------------------------------------------

    /// Capture a snapshot of `node_id`'s current state and store it as an
    /// artifact.
    ///
    /// Returns the artifact ID of the snapshot record.
    #[instrument(skip(self, state), fields(node_id = %node_id, dimension = %dimension_id))]
    pub fn snapshot_node(
        &self,
        node_id: Uuid,
        state: serde_json::Value,
        task_id: TaskId,
        dimension_id: DimensionId,
    ) -> ArtifactId {
        let artifact_id = ArtifactId::new();
        let snap = NodeSnapshot {
            artifact_id,
            node_id,
            state,
            captured_at_ms: now_ms(),
        };

        {
            let mut snaps = self.snapshots.lock().expect("mesh snapshot lock poisoned");
            snaps.insert(artifact_id, snap);
        }

        info!(artifact_id = %artifact_id, node_id = %node_id, "node snapshot captured");
        self.action_log.append(ActionLogEntry::new(
            EventType::ArtifactSnapshot,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "artifact_id": artifact_id.to_string(),
                "node_id": node_id,
            }),
        ));

        self.notify(artifact_id);
        artifact_id
    }

    /// Retrieve a previously captured node snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::NotFound`] if the ID is unknown.
    pub fn get_snapshot(&self, id: ArtifactId) -> Result<NodeSnapshot, MeshError> {
        self.snapshots
            .lock()
            .expect("mesh snapshot lock poisoned")
            .get(&id)
            .cloned()
            .ok_or(MeshError::NotFound(id))
    }

    // ------------------------------------------------------------------
    // Patch
    // ------------------------------------------------------------------

    /// Record a diff/patch between `before` and `after` states.
    ///
    /// Returns the artifact ID of the patch record.
    #[instrument(skip(self, before, after, ops), fields(dimension = %dimension_id))]
    pub fn patch(
        &self,
        before: serde_json::Value,
        after: serde_json::Value,
        ops: Vec<PatchOp>,
        task_id: TaskId,
        dimension_id: DimensionId,
    ) -> ArtifactId {
        let artifact_id = ArtifactId::new();
        let patch = DiffPatch {
            artifact_id,
            before,
            after,
            ops,
        };

        {
            let mut patches = self.patches.lock().expect("mesh patch lock poisoned");
            patches.insert(artifact_id, patch);
        }

        info!(artifact_id = %artifact_id, "mesh diff patch recorded");
        self.action_log.append(ActionLogEntry::new(
            EventType::ArtifactPatched,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "artifact_id": artifact_id.to_string(),
            }),
        ));

        self.notify(artifact_id);
        artifact_id
    }

    /// Retrieve a recorded diff/patch.
    ///
    /// # Errors
    ///
    /// Returns [`MeshError::NotFound`] if the ID is unknown.
    pub fn get_patch(&self, id: ArtifactId) -> Result<DiffPatch, MeshError> {
        self.patches
            .lock()
            .expect("mesh patch lock poisoned")
            .get(&id)
            .cloned()
            .ok_or(MeshError::NotFound(id))
    }

    // ------------------------------------------------------------------
    // Subscription
    // ------------------------------------------------------------------

    /// Subscribe to [`ArtifactId`] notifications for every new write.
    pub fn subscribe(&self) -> broadcast::Receiver<ArtifactId> {
        self.tx.subscribe()
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn notify(&self, id: ArtifactId) {
        if let Err(e) = self.tx.send(id) {
            if self.tx.receiver_count() > 0 {
                warn!("mesh broadcast failed: {e}");
            }
        }
    }

    /// Number of artifacts currently in the store.
    pub fn artifact_count(&self) -> usize {
        self.artifacts.lock().expect("mesh lock poisoned").len()
    }
}

// ---------------------------------------------------------------------------
// Builder helper
// ---------------------------------------------------------------------------

/// Convenience builder for constructing [`MeshArtifact`] instances.
pub struct MeshArtifactBuilder {
    artifact: MeshArtifact,
}

impl MeshArtifactBuilder {
    /// Start building a new artifact for the given dimension/task.
    pub fn new(dimension_id: DimensionId, task_id: TaskId) -> Self {
        Self {
            artifact: MeshArtifact {
                id: ArtifactId::new(),
                dimension_id,
                task_id,
                node_id: None,
                content_type: "application/json".to_owned(),
                payload: serde_json::Value::Null,
                provenance: ArtifactProvenance {
                    producing_task_id: task_id,
                    producing_agent_id: None,
                    producing_node_id: None,
                    controller_id: None,
                    schema_version: "1.0.0".to_owned(),
                },
                created_at_ms: 0,
                immutability_tier: 1,
                consumed: false,
            },
        }
    }

    /// Set the node ID for this artifact.
    pub fn node_id(mut self, id: Uuid) -> Self {
        self.artifact.node_id = Some(id);
        self
    }

    /// Set the content type.
    pub fn content_type(mut self, ct: impl Into<String>) -> Self {
        self.artifact.content_type = ct.into();
        self
    }

    /// Set the payload.
    pub fn payload(mut self, payload: serde_json::Value) -> Self {
        self.artifact.payload = payload;
        self
    }

    /// Set the immutability tier (0=ephemeral, 1=session, 2=persistent).
    pub fn immutability_tier(mut self, tier: u8) -> Self {
        self.artifact.immutability_tier = tier;
        self
    }

    /// Finalise and return the [`MeshArtifact`].
    pub fn build(self) -> MeshArtifact {
        self.artifact
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> Arc<MeshArtifactStore> {
        let log = ActionLog::new(32);
        MeshArtifactStore::new(log, 16)
    }

    fn make_artifact(dim: DimensionId, task: TaskId) -> MeshArtifact {
        MeshArtifactBuilder::new(dim, task)
            .payload(serde_json::json!({ "hello": "mesh" }))
            .build()
    }

    #[test]
    fn produce_and_consume() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();

        let artifact = make_artifact(dim, task);
        let id = store.produce(artifact);

        let consumed = store.consume(id).unwrap();
        assert_eq!(consumed.id, id);
        assert!(consumed.consumed);
    }

    #[test]
    fn consume_twice_fails() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();

        let id = store.produce(make_artifact(dim, task));
        store.consume(id).unwrap();

        let err = store.consume(id);
        assert!(matches!(err, Err(MeshError::AlreadyConsumed(_))));
    }

    #[test]
    fn consume_unknown_fails() {
        let store = make_store();
        let err = store.consume(ArtifactId::new());
        assert!(matches!(err, Err(MeshError::NotFound(_))));
    }

    #[test]
    fn snapshot_node_roundtrip() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let node_id = Uuid::new_v4();
        let state = serde_json::json!({ "x": 100, "y": 200 });

        let snap_id = store.snapshot_node(node_id, state.clone(), task, dim);
        let snap = store.get_snapshot(snap_id).unwrap();

        assert_eq!(snap.node_id, node_id);
        assert_eq!(snap.state, state);
    }

    #[test]
    fn patch_roundtrip() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();

        let before = serde_json::json!({ "color": "red" });
        let after = serde_json::json!({ "color": "blue" });
        let ops = vec![PatchOp::Replace {
            path: "/color".to_owned(),
            old: serde_json::json!("red"),
            new: serde_json::json!("blue"),
        }];

        let patch_id = store.patch(before.clone(), after.clone(), ops, task, dim);
        let patch = store.get_patch(patch_id).unwrap();

        assert_eq!(patch.before, before);
        assert_eq!(patch.after, after);
        assert_eq!(patch.ops.len(), 1);
    }

    #[tokio::test]
    async fn subscribe_receives_produce_notification() {
        let store = make_store();
        let mut rx = store.subscribe();

        let dim = DimensionId::new();
        let task = TaskId::new();
        let id = store.produce(make_artifact(dim, task));

        let received = rx.recv().await.unwrap();
        assert_eq!(received, id);
    }
}
