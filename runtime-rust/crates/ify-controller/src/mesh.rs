//! Mesh data canvas: produce, consume, snapshot, diff/patch, routing, and
//! replication.
//!
//! Satisfies Epic B requirement:
//! > Implement mesh-artifact write path (produce/consume artifacts, node
//! > snapshots, diff patches).
//!
//! Expanded for Epic M to add schema registration, filtered subscriptions,
//! conflict resolution, batching, indexing, garbage collection, and replication
//! support for the mesh data canvas.
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

use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// The schema version string was malformed.
    #[error("invalid schema version '{0}'")]
    InvalidSchemaVersion(String),
    /// The schema version has not been registered for this content type.
    #[error("schema for content type '{content_type}' with version '{version}' is not registered")]
    SchemaNotRegistered {
        /// Content type of the schema.
        content_type: String,
        /// Version string that was requested.
        version: String,
    },
    /// The schema version is incompatible with the registered schema.
    #[error("schema version mismatch for '{content_type}': expected '{expected}', got '{actual}'")]
    SchemaVersionMismatch {
        /// Content type of the schema.
        content_type: String,
        /// Expected version.
        expected: String,
        /// Actual version supplied.
        actual: String,
    },
    /// Concurrent edits detected for a node.
    #[error("concurrent edit detected for node {node_id}: expected {expected}, current {current}")]
    ConcurrentEdit {
        /// Node being edited.
        node_id: Uuid,
        /// Expected revision.
        expected: u64,
        /// Current revision.
        current: u64,
    },
    /// Batched artifacts must share a common metadata field.
    #[error("batch artifacts must share the same {field}")]
    BatchMismatch {
        /// Field that differed across artifacts.
        field: &'static str,
    },
    /// Batch operations require at least one artifact.
    #[error("batch must contain at least one artifact")]
    EmptyBatch,
    /// The artifact already exists in the mesh store.
    #[error("artifact {0} already exists in mesh store")]
    AlreadyExists(ArtifactId),
}

// ---------------------------------------------------------------------------
// Schema registry
// ---------------------------------------------------------------------------

/// Semantic version for mesh artifact schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SchemaVersion {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
    /// Patch version.
    pub patch: u32,
}

impl SchemaVersion {
    /// Parse a `MAJOR.MINOR.PATCH` string into a [`SchemaVersion`].
    pub fn parse(value: &str) -> Result<Self, MeshError> {
        let parts: Vec<&str> = value.split('.').collect();
        if parts.len() != 3 {
            return Err(MeshError::InvalidSchemaVersion(value.to_owned()));
        }
        let major = parts[0]
            .parse::<u32>()
            .map_err(|_| MeshError::InvalidSchemaVersion(value.to_owned()))?;
        let minor = parts[1]
            .parse::<u32>()
            .map_err(|_| MeshError::InvalidSchemaVersion(value.to_owned()))?;
        let patch = parts[2]
            .parse::<u32>()
            .map_err(|_| MeshError::InvalidSchemaVersion(value.to_owned()))?;
        Ok(Self { major, minor, patch })
    }
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A registered artifact schema definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDefinition {
    /// Content type this schema applies to.
    pub content_type: String,
    /// Schema version.
    pub version: SchemaVersion,
    /// Human-readable description.
    pub description: String,
    /// Registration timestamp.
    pub registered_at_ms: u64,
}

/// Registry of mesh artifact schemas.
#[derive(Debug, Default)]
pub struct MeshSchemaRegistry {
    schemas: HashMap<String, BTreeMap<SchemaVersion, SchemaDefinition>>,
}

impl MeshSchemaRegistry {
    /// Register a schema definition.
    pub fn register_schema(
        &mut self,
        content_type: impl Into<String>,
        version: &str,
        description: impl Into<String>,
    ) -> Result<SchemaDefinition, MeshError> {
        let content_type = content_type.into();
        let version = SchemaVersion::parse(version)?;
        let definition = SchemaDefinition {
            content_type: content_type.clone(),
            version,
            description: description.into(),
            registered_at_ms: now_ms(),
        };
        let entry = self.schemas.entry(content_type).or_default();
        entry.insert(version, definition.clone());
        Ok(definition)
    }

    /// Return a schema definition, if registered.
    pub fn schema(&self, content_type: &str, version: &SchemaVersion) -> Option<&SchemaDefinition> {
        self.schemas.get(content_type).and_then(|m| m.get(version))
    }

    /// Validate that a schema version is registered and compatible.
    pub fn validate(&self, content_type: &str, version: &SchemaVersion) -> Result<(), MeshError> {
        let versions = self.schemas.get(content_type).ok_or_else(|| {
            MeshError::SchemaNotRegistered {
                content_type: content_type.to_owned(),
                version: version.to_string(),
            }
        })?;
        if versions.contains_key(version) {
            return Ok(());
        }
        if let Some((expected, _)) = versions.iter().rev().find(|(v, _)| v.major == version.major) {
            return Err(MeshError::SchemaVersionMismatch {
                content_type: content_type.to_owned(),
                expected: expected.to_string(),
                actual: version.to_string(),
            });
        }
        Err(MeshError::SchemaNotRegistered {
            content_type: content_type.to_owned(),
            version: version.to_string(),
        })
    }
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
    /// Tags used for indexing and routing.
    pub tags: Vec<String>,
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
    /// Dimension this snapshot belongs to.
    pub dimension_id: DimensionId,
    /// Task that triggered the snapshot.
    pub task_id: TaskId,
    /// The node this snapshot was taken from.
    pub node_id: Uuid,
    /// Serialized node state at capture time.
    pub state: serde_json::Value,
    /// Provenance chain.
    pub provenance: ArtifactProvenance,
    /// Unix epoch milliseconds of capture.
    pub captured_at_ms: u64,
    /// Node revision at capture time.
    pub revision: u64,
}

// ---------------------------------------------------------------------------
// DiffPatch
// ---------------------------------------------------------------------------

/// A diff/patch record capturing the delta between two artifact states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffPatch {
    /// The artifact ID for this patch record.
    pub artifact_id: ArtifactId,
    /// Dimension this patch belongs to.
    pub dimension_id: DimensionId,
    /// Task that produced the patch.
    pub task_id: TaskId,
    /// Node being patched.
    pub node_id: Uuid,
    /// Provenance chain.
    pub provenance: ArtifactProvenance,
    /// Unix epoch milliseconds when this patch was recorded.
    pub created_at_ms: u64,
    /// Base revision before applying this patch.
    pub base_revision: u64,
    /// New revision after applying this patch.
    pub new_revision: u64,
    /// Whether this patch resolved a conflict.
    pub conflict: bool,
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
// Notifications and filters
// ---------------------------------------------------------------------------

/// Kind of mesh notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshNotificationKind {
    /// A new artifact was produced.
    Produced,
    /// An artifact was consumed.
    Consumed,
    /// A node snapshot was captured.
    Snapshot,
    /// A diff patch was recorded.
    Patch,
    /// A batch of artifacts was produced.
    BatchProduced,
    /// An artifact was replicated from another runtime.
    Replicated,
}

/// Metadata for a mesh notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshNotification {
    /// Artifact IDs affected by this event.
    pub ids: Vec<ArtifactId>,
    /// Notification kind.
    pub kind: MeshNotificationKind,
    /// Dimension associated with the event.
    pub dimension_id: DimensionId,
    /// Task associated with the event.
    pub task_id: TaskId,
    /// Node associated with the event.
    pub node_id: Option<Uuid>,
    /// Tags associated with the event (if any).
    pub tags: Vec<String>,
    /// Content type for the event payload.
    pub content_type: String,
}

impl MeshNotification {
    fn single(
        id: ArtifactId,
        kind: MeshNotificationKind,
        dimension_id: DimensionId,
        task_id: TaskId,
        node_id: Option<Uuid>,
        tags: Vec<String>,
        content_type: String,
    ) -> Self {
        Self {
            ids: vec![id],
            kind,
            dimension_id,
            task_id,
            node_id,
            tags,
            content_type,
        }
    }

    fn batch(
        ids: Vec<ArtifactId>,
        dimension_id: DimensionId,
        task_id: TaskId,
        node_id: Option<Uuid>,
        tags: Vec<String>,
        content_type: String,
    ) -> Self {
        Self {
            ids,
            kind: MeshNotificationKind::BatchProduced,
            dimension_id,
            task_id,
            node_id,
            tags,
            content_type,
        }
    }
}

/// Filter for mesh notifications.
#[derive(Debug, Clone, Default)]
pub struct SubscriptionFilter {
    /// Only receive notifications for this dimension.
    pub dimension_id: Option<DimensionId>,
    /// Only receive notifications for this task.
    pub task_id: Option<TaskId>,
    /// Only receive notifications for this node.
    pub node_id: Option<Uuid>,
    /// Only receive notifications that include this tag.
    pub tag: Option<String>,
    /// Only receive notifications for this content type.
    pub content_type: Option<String>,
    /// Restrict to specific notification kinds.
    pub kinds: Option<Vec<MeshNotificationKind>>,
}

impl SubscriptionFilter {
    fn matches(&self, note: &MeshNotification) -> bool {
        if let Some(dim) = self.dimension_id {
            if note.dimension_id != dim {
                return false;
            }
        }
        if let Some(task) = self.task_id {
            if note.task_id != task {
                return false;
            }
        }
        if let Some(node) = self.node_id {
            if note.node_id != Some(node) {
                return false;
            }
        }
        if let Some(tag) = &self.tag {
            if !note.tags.iter().any(|t| t == tag) {
                return false;
            }
        }
        if let Some(content_type) = &self.content_type {
            if &note.content_type != content_type {
                return false;
            }
        }
        if let Some(kinds) = &self.kinds {
            if !kinds.contains(&note.kind) {
                return false;
            }
        }
        true
    }
}

/// Subscription wrapper that applies a [`SubscriptionFilter`].
pub struct MeshSubscription {
    filter: SubscriptionFilter,
    receiver: broadcast::Receiver<MeshNotification>,
}

impl MeshSubscription {
    /// Wait for the next notification that matches the filter.
    pub async fn recv(&mut self) -> Result<MeshNotification, broadcast::error::RecvError> {
        loop {
            let note = self.receiver.recv().await?;
            if self.filter.matches(&note) {
                return Ok(note);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Conflict handling
// ---------------------------------------------------------------------------

/// Conflict resolution strategy for concurrent mesh edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStrategy {
    /// Reject conflicting edits.
    Reject,
    /// Accept the incoming edit and mark it as conflicted.
    LastWriteWins,
}

/// Result of applying a patch with revision tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchApplyResult {
    /// Artifact ID of the stored patch.
    pub artifact_id: ArtifactId,
    /// New revision after applying the patch.
    pub new_revision: u64,
    /// Whether the patch resolved a conflict.
    pub conflict: bool,
}

// ---------------------------------------------------------------------------
// Mesh node state
// ---------------------------------------------------------------------------

/// Latest known mesh data state for a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshNodeState {
    /// Node identifier.
    pub node_id: Uuid,
    /// Current revision for the node.
    pub revision: u64,
    /// Last artifact produced for the node.
    pub last_artifact_id: Option<ArtifactId>,
    /// Last snapshot recorded for the node.
    pub last_snapshot_id: Option<ArtifactId>,
    /// Last patch recorded for the node.
    pub last_patch_id: Option<ArtifactId>,
    /// Last update timestamp.
    pub updated_at_ms: u64,
}

impl MeshNodeState {
    fn new(node_id: Uuid) -> Self {
        Self {
            node_id,
            revision: 0,
            last_artifact_id: None,
            last_snapshot_id: None,
            last_patch_id: None,
            updated_at_ms: now_ms(),
        }
    }
}

// ---------------------------------------------------------------------------
// Artifact indexing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct ArtifactIndex {
    by_dimension: HashMap<DimensionId, Vec<ArtifactId>>,
    by_task: HashMap<TaskId, Vec<ArtifactId>>,
    by_node: HashMap<Uuid, Vec<ArtifactId>>,
    by_agent: HashMap<String, Vec<ArtifactId>>,
    by_tag: HashMap<String, Vec<ArtifactId>>,
}

impl ArtifactIndex {
    fn insert(&mut self, artifact: &MeshArtifact) {
        self.by_dimension
            .entry(artifact.dimension_id)
            .or_default()
            .push(artifact.id);
        self.by_task
            .entry(artifact.task_id)
            .or_default()
            .push(artifact.id);
        if let Some(node_id) = artifact.node_id {
            self.by_node.entry(node_id).or_default().push(artifact.id);
        }
        if let Some(agent_id) = artifact.provenance.producing_agent_id.as_ref() {
            self.by_agent
                .entry(agent_id.clone())
                .or_default()
                .push(artifact.id);
        }
        for tag in &artifact.tags {
            self.by_tag.entry(tag.clone()).or_default().push(artifact.id);
        }
    }

    fn remove(&mut self, artifact: &MeshArtifact) {
        if let Some(ids) = self.by_dimension.get_mut(&artifact.dimension_id) {
            ids.retain(|id| id != &artifact.id);
        }
        if let Some(ids) = self.by_task.get_mut(&artifact.task_id) {
            ids.retain(|id| id != &artifact.id);
        }
        if let Some(node_id) = artifact.node_id {
            if let Some(ids) = self.by_node.get_mut(&node_id) {
                ids.retain(|id| id != &artifact.id);
            }
        }
        if let Some(agent_id) = artifact.provenance.producing_agent_id.as_ref() {
            if let Some(ids) = self.by_agent.get_mut(agent_id) {
                ids.retain(|id| id != &artifact.id);
            }
        }
        for tag in &artifact.tags {
            if let Some(ids) = self.by_tag.get_mut(tag) {
                ids.retain(|id| id != &artifact.id);
            }
        }
    }

    fn by_dimension(&self, dimension_id: DimensionId) -> Vec<ArtifactId> {
        self.by_dimension
            .get(&dimension_id)
            .cloned()
            .unwrap_or_default()
    }

    fn by_task(&self, task_id: TaskId) -> Vec<ArtifactId> {
        self.by_task.get(&task_id).cloned().unwrap_or_default()
    }

    fn by_node(&self, node_id: Uuid) -> Vec<ArtifactId> {
        self.by_node.get(&node_id).cloned().unwrap_or_default()
    }

    fn by_agent(&self, agent_id: &str) -> Vec<ArtifactId> {
        self.by_agent
            .get(agent_id)
            .cloned()
            .unwrap_or_default()
    }

    fn by_tag(&self, tag: &str) -> Vec<ArtifactId> {
        self.by_tag.get(tag).cloned().unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Garbage collection
// ---------------------------------------------------------------------------

/// Garbage collection policy for mesh artifacts.
#[derive(Debug, Clone)]
pub struct GarbageCollectionPolicy {
    /// Active dimensions that should be retained.
    pub active_dimensions: HashSet<DimensionId>,
    /// Remove consumed ephemeral artifacts.
    pub remove_consumed_ephemeral: bool,
    /// Remove orphaned snapshots for nodes that no longer exist.
    pub remove_orphaned_nodes: bool,
    /// Remove artifacts created before this timestamp.
    pub expire_before_ms: Option<u64>,
}

impl Default for GarbageCollectionPolicy {
    fn default() -> Self {
        Self {
            active_dimensions: HashSet::new(),
            remove_consumed_ephemeral: true,
            remove_orphaned_nodes: true,
            expire_before_ms: None,
        }
    }
}

/// Report produced by mesh garbage collection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GarbageCollectionReport {
    /// Number of artifacts removed.
    pub artifacts_removed: usize,
    /// Number of snapshots removed.
    pub snapshots_removed: usize,
    /// Number of patches removed.
    pub patches_removed: usize,
}

// ---------------------------------------------------------------------------
// Replication
// ---------------------------------------------------------------------------

/// Simple in-process replication link between two mesh stores.
#[derive(Debug, Clone)]
pub struct MeshReplicator {
    source: Arc<MeshArtifactStore>,
    target: Arc<MeshArtifactStore>,
}

impl MeshReplicator {
    /// Create a new replication link.
    pub fn new(source: Arc<MeshArtifactStore>, target: Arc<MeshArtifactStore>) -> Self {
        Self { source, target }
    }

    /// Replicate a single artifact by ID.
    pub fn replicate_artifact(&self, id: ArtifactId) -> Result<ArtifactId, MeshError> {
        let artifact = self.source.get_artifact(id)?;
        self.target.insert_replicated_artifact(artifact)?;
        Ok(id)
    }

    /// Replicate a snapshot by ID.
    pub fn replicate_snapshot(&self, id: ArtifactId) -> Result<ArtifactId, MeshError> {
        let snapshot = self.source.get_snapshot(id)?;
        self.target.insert_replicated_snapshot(snapshot)?;
        Ok(id)
    }

    /// Replicate a patch by ID.
    pub fn replicate_patch(&self, id: ArtifactId) -> Result<ArtifactId, MeshError> {
        let patch = self.source.get_patch(id)?;
        self.target.insert_replicated_patch(patch)?;
        Ok(id)
    }

    /// Replicate all artifacts referenced by a notification.
    pub fn replicate_notification(&self, notification: &MeshNotification) -> Result<usize, MeshError> {
        let mut replicated = 0;
        for id in &notification.ids {
            match notification.kind {
                MeshNotificationKind::Snapshot => self.replicate_snapshot(*id)?,
                MeshNotificationKind::Patch => self.replicate_patch(*id)?,
                _ => self.replicate_artifact(*id)?,
            };
            replicated += 1;
        }
        Ok(replicated)
    }
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
    index: Mutex<ArtifactIndex>,
    schema_registry: Mutex<MeshSchemaRegistry>,
    node_states: Mutex<HashMap<Uuid, MeshNodeState>>,
    action_log: Arc<ActionLog>,
    tx: broadcast::Sender<ArtifactId>,
    notification_tx: broadcast::Sender<MeshNotification>,
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
        let (notification_tx, _) = broadcast::channel(channel_capacity.max(1));
        let mut registry = MeshSchemaRegistry::default();
        let _ = registry.register_schema("application/json", "1.0.0", "Default JSON schema");
        Arc::new(Self {
            artifacts: Mutex::new(HashMap::new()),
            snapshots: Mutex::new(HashMap::new()),
            patches: Mutex::new(HashMap::new()),
            index: Mutex::new(ArtifactIndex::default()),
            schema_registry: Mutex::new(registry),
            node_states: Mutex::new(HashMap::new()),
            action_log,
            tx,
            notification_tx,
        })
    }

    // ------------------------------------------------------------------
    // Schema registry
    // ------------------------------------------------------------------

    /// Register a mesh artifact schema.
    pub fn register_schema(
        &self,
        content_type: impl Into<String>,
        version: &str,
        description: impl Into<String>,
    ) -> Result<SchemaDefinition, MeshError> {
        self.schema_registry
            .lock()
            .expect("mesh schema registry lock poisoned")
            .register_schema(content_type, version, description)
    }

    /// Validate a schema version against the registry.
    pub fn validate_schema(&self, content_type: &str, version: &str) -> Result<(), MeshError> {
        let parsed = SchemaVersion::parse(version)?;
        self.schema_registry
            .lock()
            .expect("mesh schema registry lock poisoned")
            .validate(content_type, &parsed)
    }

    /// Return a registered schema definition if it exists.
    pub fn schema_definition(&self, content_type: &str, version: &str) -> Option<SchemaDefinition> {
        let parsed = SchemaVersion::parse(version).ok()?;
        self.schema_registry
            .lock()
            .expect("mesh schema registry lock poisoned")
            .schema(content_type, &parsed)
            .cloned()
    }

    // ------------------------------------------------------------------
    // Mesh data representation
    // ------------------------------------------------------------------

    /// Return the latest known mesh state for a node.
    pub fn node_state(&self, node_id: Uuid) -> Option<MeshNodeState> {
        self.node_states
            .lock()
            .expect("mesh node state lock poisoned")
            .get(&node_id)
            .cloned()
    }

    /// Return all mesh node states currently tracked.
    pub fn node_states(&self) -> Vec<MeshNodeState> {
        self.node_states
            .lock()
            .expect("mesh node state lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    // ------------------------------------------------------------------
    // Produce
    // ------------------------------------------------------------------

    /// Produce an artifact after validating its schema.
    pub fn produce_validated(&self, artifact: MeshArtifact) -> Result<ArtifactId, MeshError> {
        let mut artifact = artifact;
        self.prepare_artifact(&mut artifact);
        self.validate_schema(&artifact.content_type, &artifact.provenance.schema_version)?;
        Ok(self.produce(artifact))
    }

    // ------------------------------------------------------------------
    // Produce
    // ------------------------------------------------------------------

    /// Produce an artifact and commit it to the store.
    ///
    /// Returns the assigned [`ArtifactId`].
    #[instrument(skip(self, artifact), fields(dimension = %artifact.dimension_id, task_id = %artifact.task_id))]
    pub fn produce(&self, mut artifact: MeshArtifact) -> ArtifactId {
        self.prepare_artifact(&mut artifact);
        let id = artifact.id;
        artifact.created_at_ms = now_ms();
        artifact.consumed = false;

        let dimension_id = artifact.dimension_id;
        let task_id = artifact.task_id;
        let node_id = artifact.node_id;
        let tags = artifact.tags.clone();
        let content_type = artifact.content_type.clone();

        {
            let mut arts = self.artifacts.lock().expect("mesh lock poisoned");
            arts.insert(id, artifact.clone());
        }
        self.index
            .lock()
            .expect("mesh index lock poisoned")
            .insert(&artifact);
        if let Some(node_id) = node_id {
            self.touch_node_state(node_id, |state| {
                state.last_artifact_id = Some(id);
            });
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
                "node_id": node_id.map(|v| v.to_string()),
                "tags": tags,
            }),
        ));

        self.notify(MeshNotification::single(
            id,
            MeshNotificationKind::Produced,
            dimension_id,
            task_id,
            node_id,
            tags,
            content_type,
        ));
        id
    }

    /// Produce a batch of artifacts with shared metadata.
    pub fn produce_batch(&self, mut artifacts: Vec<MeshArtifact>) -> Result<Vec<ArtifactId>, MeshError> {
        if artifacts.is_empty() {
            return Err(MeshError::EmptyBatch);
        }
        for artifact in &mut artifacts {
            self.prepare_artifact(artifact);
        }
        let base = &artifacts[0];
        let dimension_id = base.dimension_id;
        let task_id = base.task_id;
        let node_id = base.node_id;
        let content_type = base.content_type.clone();
        let tags = base.tags.clone();
        for artifact in &artifacts {
            if artifact.dimension_id != dimension_id {
                return Err(MeshError::BatchMismatch { field: "dimension_id" });
            }
            if artifact.task_id != task_id {
                return Err(MeshError::BatchMismatch { field: "task_id" });
            }
            if artifact.node_id != node_id {
                return Err(MeshError::BatchMismatch { field: "node_id" });
            }
            if artifact.content_type != content_type {
                return Err(MeshError::BatchMismatch { field: "content_type" });
            }
            if artifact.tags != tags {
                return Err(MeshError::BatchMismatch { field: "tags" });
            }
        }

        let mut ids = Vec::with_capacity(artifacts.len());
        {
            let mut arts = self.artifacts.lock().expect("mesh lock poisoned");
            let mut index = self.index.lock().expect("mesh index lock poisoned");
            for mut artifact in artifacts {
                artifact.created_at_ms = now_ms();
                artifact.consumed = false;
                let id = artifact.id;
                ids.push(id);
                arts.insert(id, artifact.clone());
                index.insert(&artifact);
            }
        }

        if let Some(node_id) = node_id {
            if let Some(last_id) = ids.last().copied() {
                self.touch_node_state(node_id, |state| {
                    state.last_artifact_id = Some(last_id);
                });
            }
        }

        self.action_log.append(ActionLogEntry::new(
            EventType::ArtifactProduced,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "artifact_ids": ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                "dimension_id": dimension_id.to_string(),
                "node_id": node_id.map(|v| v.to_string()),
                "count": ids.len(),
            }),
        ));

        self.notify(MeshNotification::batch(
            ids.clone(),
            dimension_id,
            task_id,
            node_id,
            tags,
            content_type,
        ));

        Ok(ids)
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

        let note = MeshNotification::single(
            id,
            MeshNotificationKind::Consumed,
            result.dimension_id,
            result.task_id,
            result.node_id,
            result.tags.clone(),
            result.content_type.clone(),
        );

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

        self.notify(note);
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
        let revision = self.current_revision(node_id);
        let provenance = ArtifactProvenance {
            producing_task_id: task_id,
            producing_agent_id: None,
            producing_node_id: Some(node_id),
            controller_id: None,
            schema_version: "1.0.0".to_owned(),
        };
        let snap = NodeSnapshot {
            artifact_id,
            dimension_id,
            task_id,
            node_id,
            state,
            provenance,
            captured_at_ms: now_ms(),
            revision,
        };

        {
            let mut snaps = self.snapshots.lock().expect("mesh snapshot lock poisoned");
            snaps.insert(artifact_id, snap);
        }

        self.touch_node_state(node_id, |state| {
            state.last_snapshot_id = Some(artifact_id);
        });

        info!(artifact_id = %artifact_id, node_id = %node_id, "node snapshot captured");
        self.action_log.append(ActionLogEntry::new(
            EventType::ArtifactSnapshot,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "artifact_id": artifact_id.to_string(),
                "node_id": node_id,
                "revision": revision,
            }),
        ));

        self.notify(MeshNotification::single(
            artifact_id,
            MeshNotificationKind::Snapshot,
            dimension_id,
            task_id,
            Some(node_id),
            Vec::new(),
            "application/json".to_owned(),
        ));
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
        node_id: Uuid,
        before: serde_json::Value,
        after: serde_json::Value,
        ops: Vec<PatchOp>,
        task_id: TaskId,
        dimension_id: DimensionId,
    ) -> ArtifactId {
        let before_fallback = before.clone();
        let after_fallback = after.clone();
        let ops_fallback = ops.clone();
        match self.patch_with_revision(
            node_id,
            before,
            after,
            ops,
            task_id,
            dimension_id,
            self.current_revision(node_id),
            ConflictStrategy::LastWriteWins,
        ) {
            Ok(result) => result.artifact_id,
            Err(err) => {
                warn!(error = %err, "mesh patch fell back to force apply");
                self.force_patch(
                    node_id,
                    before_fallback,
                    after_fallback,
                    ops_fallback,
                    task_id,
                    dimension_id,
                )
            }
        }
    }

    /// Record a diff/patch with revision validation.
    #[allow(clippy::too_many_arguments)]
    pub fn patch_with_revision(
        &self,
        node_id: Uuid,
        before: serde_json::Value,
        after: serde_json::Value,
        ops: Vec<PatchOp>,
        task_id: TaskId,
        dimension_id: DimensionId,
        expected_revision: u64,
        strategy: ConflictStrategy,
    ) -> Result<PatchApplyResult, MeshError> {
        let artifact_id = ArtifactId::new();
        let (base_revision, new_revision, conflict) =
            self.advance_revision(node_id, expected_revision, strategy, Some(artifact_id))?;
        let patch = DiffPatch {
            artifact_id,
            dimension_id,
            task_id,
            node_id,
            provenance: ArtifactProvenance {
                producing_task_id: task_id,
                producing_agent_id: None,
                producing_node_id: Some(node_id),
                controller_id: None,
                schema_version: "1.0.0".to_owned(),
            },
            created_at_ms: now_ms(),
            base_revision,
            new_revision,
            conflict,
            before,
            after,
            ops,
        };

        self.store_patch(patch);
        Ok(PatchApplyResult {
            artifact_id,
            new_revision,
            conflict,
        })
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

    /// Subscribe to detailed mesh notifications.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<MeshNotification> {
        self.notification_tx.subscribe()
    }

    /// Subscribe with a filter applied to mesh notifications.
    pub fn subscribe_filtered(&self, filter: SubscriptionFilter) -> MeshSubscription {
        MeshSubscription {
            filter,
            receiver: self.notification_tx.subscribe(),
        }
    }

    // ------------------------------------------------------------------
    // Index queries
    // ------------------------------------------------------------------

    /// Return all artifacts for a dimension.
    pub fn artifacts_for_dimension(&self, dimension_id: DimensionId) -> Vec<ArtifactId> {
        self.index
            .lock()
            .expect("mesh index lock poisoned")
            .by_dimension(dimension_id)
    }

    /// Return all artifacts produced by a task.
    pub fn artifacts_for_task(&self, task_id: TaskId) -> Vec<ArtifactId> {
        self.index
            .lock()
            .expect("mesh index lock poisoned")
            .by_task(task_id)
    }

    /// Return all artifacts associated with a node.
    pub fn artifacts_for_node(&self, node_id: Uuid) -> Vec<ArtifactId> {
        self.index
            .lock()
            .expect("mesh index lock poisoned")
            .by_node(node_id)
    }

    /// Return all artifacts produced by an agent.
    pub fn artifacts_for_agent(&self, agent_id: &str) -> Vec<ArtifactId> {
        self.index
            .lock()
            .expect("mesh index lock poisoned")
            .by_agent(agent_id)
    }

    /// Return all artifacts tagged with `tag`.
    pub fn artifacts_with_tag(&self, tag: &str) -> Vec<ArtifactId> {
        self.index
            .lock()
            .expect("mesh index lock poisoned")
            .by_tag(tag)
    }

    // ------------------------------------------------------------------
    // Lookup
    // ------------------------------------------------------------------

    /// Retrieve an artifact without marking it as consumed.
    pub fn get_artifact(&self, id: ArtifactId) -> Result<MeshArtifact, MeshError> {
        self.artifacts
            .lock()
            .expect("mesh lock poisoned")
            .get(&id)
            .cloned()
            .ok_or(MeshError::NotFound(id))
    }

    // ------------------------------------------------------------------
    // Garbage collection
    // ------------------------------------------------------------------

    /// Collect mesh artifacts based on the provided policy.
    pub fn collect_garbage(&self, policy: &GarbageCollectionPolicy) -> GarbageCollectionReport {
        let mut report = GarbageCollectionReport::default();
        let mut removed_artifacts = Vec::new();
        {
            let mut arts = self.artifacts.lock().expect("mesh lock poisoned");
            arts.retain(|_, artifact| {
                let expired = policy
                    .expire_before_ms
                    .map(|cutoff| artifact.created_at_ms < cutoff)
                    .unwrap_or(false);
                let inactive_dimension = !policy.active_dimensions.is_empty()
                    && !policy.active_dimensions.contains(&artifact.dimension_id)
                    && artifact.immutability_tier <= 1;
                let consumed_ephemeral =
                    policy.remove_consumed_ephemeral && artifact.immutability_tier == 0 && artifact.consumed;
                let remove = expired || inactive_dimension || consumed_ephemeral;
                if remove {
                    removed_artifacts.push(artifact.clone());
                    report.artifacts_removed += 1;
                }
                !remove
            });
        }

        if !removed_artifacts.is_empty() {
            let mut index = self.index.lock().expect("mesh index lock poisoned");
            for artifact in &removed_artifacts {
                index.remove(artifact);
            }
        }

        {
            let mut states = self.node_states.lock().expect("mesh node state lock poisoned");
            for artifact in &removed_artifacts {
                if let Some(node_id) = artifact.node_id {
                    if let Some(state) = states.get_mut(&node_id) {
                        if state.last_artifact_id == Some(artifact.id) {
                            state.last_artifact_id = None;
                        }
                    }
                }
            }
        }

        let known_nodes: HashSet<Uuid> = self
            .node_states
            .lock()
            .expect("mesh node state lock poisoned")
            .keys()
            .copied()
            .collect();
        {
            let mut snaps = self.snapshots.lock().expect("mesh snapshot lock poisoned");
            snaps.retain(|_, snapshot| {
                let expired = policy
                    .expire_before_ms
                    .map(|cutoff| snapshot.captured_at_ms < cutoff)
                    .unwrap_or(false);
                let inactive_dimension = !policy.active_dimensions.is_empty()
                    && !policy.active_dimensions.contains(&snapshot.dimension_id);
                let orphaned = policy.remove_orphaned_nodes && !known_nodes.contains(&snapshot.node_id);
                let remove = expired || inactive_dimension || orphaned;
                if remove {
                    report.snapshots_removed += 1;
                }
                !remove
            });
        }

        {
            let mut patches = self.patches.lock().expect("mesh patch lock poisoned");
            patches.retain(|_, patch| {
                let expired = policy
                    .expire_before_ms
                    .map(|cutoff| patch.created_at_ms < cutoff)
                    .unwrap_or(false);
                let inactive_dimension = !policy.active_dimensions.is_empty()
                    && !policy.active_dimensions.contains(&patch.dimension_id);
                let remove = expired || inactive_dimension;
                if remove {
                    report.patches_removed += 1;
                }
                !remove
            });
        }

        report
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn notify(&self, notification: MeshNotification) {
        for id in &notification.ids {
            if let Err(e) = self.tx.send(*id) {
                if self.tx.receiver_count() > 0 {
                    warn!("mesh broadcast failed: {e}");
                }
            }
        }
        if let Err(e) = self.notification_tx.send(notification) {
            if self.notification_tx.receiver_count() > 0 {
                warn!("mesh notification broadcast failed: {e}");
            }
        }
    }

    fn prepare_artifact(&self, artifact: &mut MeshArtifact) {
        if artifact.content_type.trim().is_empty() {
            artifact.content_type = "application/json".to_owned();
        }
        if artifact.provenance.schema_version.trim().is_empty() {
            artifact.provenance.schema_version = "1.0.0".to_owned();
        }
        artifact.provenance.producing_task_id = artifact.task_id;
        if artifact.provenance.producing_node_id.is_none() {
            artifact.provenance.producing_node_id = artifact.node_id;
        }
        if artifact.tags.len() > 1 {
            let mut sorted = true;
            let mut has_dupe = false;
            for window in artifact.tags.windows(2) {
                if window[0] > window[1] {
                    sorted = false;
                    break;
                }
                if window[0] == window[1] {
                    has_dupe = true;
                }
            }
            if !sorted {
                artifact.tags.sort();
                artifact.tags.dedup();
            } else if has_dupe {
                artifact.tags.dedup();
            }
        }
    }

    fn current_revision(&self, node_id: Uuid) -> u64 {
        self.node_states
            .lock()
            .expect("mesh node state lock poisoned")
            .get(&node_id)
            .map(|state| state.revision)
            .unwrap_or(0)
    }

    fn touch_node_state<F>(&self, node_id: Uuid, update: F)
    where
        F: FnOnce(&mut MeshNodeState),
    {
        let mut states = self.node_states.lock().expect("mesh node state lock poisoned");
        let state = states.entry(node_id).or_insert_with(|| MeshNodeState::new(node_id));
        update(state);
        state.updated_at_ms = now_ms();
    }

    fn advance_revision(
        &self,
        node_id: Uuid,
        expected_revision: u64,
        strategy: ConflictStrategy,
        patch_id: Option<ArtifactId>,
    ) -> Result<(u64, u64, bool), MeshError> {
        let mut states = self.node_states.lock().expect("mesh node state lock poisoned");
        let state = states.entry(node_id).or_insert_with(|| MeshNodeState::new(node_id));
        let base_revision = state.revision;
        let conflict = base_revision != expected_revision;
        if conflict && strategy == ConflictStrategy::Reject {
            return Err(MeshError::ConcurrentEdit {
                node_id,
                expected: expected_revision,
                current: base_revision,
            });
        }
        let new_revision = base_revision.saturating_add(1);
        state.revision = new_revision;
        if let Some(patch_id) = patch_id {
            state.last_patch_id = Some(patch_id);
        }
        state.updated_at_ms = now_ms();
        Ok((base_revision, new_revision, conflict))
    }

    fn force_patch(
        &self,
        node_id: Uuid,
        before: serde_json::Value,
        after: serde_json::Value,
        ops: Vec<PatchOp>,
        task_id: TaskId,
        dimension_id: DimensionId,
    ) -> ArtifactId {
        let artifact_id = ArtifactId::new();
        let (base_revision, new_revision, conflict) = match self.advance_revision(
            node_id,
            self.current_revision(node_id),
            ConflictStrategy::LastWriteWins,
            Some(artifact_id),
        ) {
            Ok(values) => values,
            Err(err) => {
                warn!(error = %err, "mesh patch revision advance failed");
                let current = self.current_revision(node_id);
                (current, current.saturating_add(1), true)
            }
        };
        let patch = DiffPatch {
            artifact_id,
            dimension_id,
            task_id,
            node_id,
            provenance: ArtifactProvenance {
                producing_task_id: task_id,
                producing_agent_id: None,
                producing_node_id: Some(node_id),
                controller_id: None,
                schema_version: "1.0.0".to_owned(),
            },
            created_at_ms: now_ms(),
            base_revision,
            new_revision,
            conflict,
            before,
            after,
            ops,
        };
        self.store_patch(patch);
        artifact_id
    }

    fn store_patch(&self, patch: DiffPatch) {
        let artifact_id = patch.artifact_id;
        let dimension_id = patch.dimension_id;
        let task_id = patch.task_id;
        let node_id = patch.node_id;
        let base_revision = patch.base_revision;
        let new_revision = patch.new_revision;
        let conflict = patch.conflict;
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
                "node_id": node_id,
                "base_revision": base_revision,
                "new_revision": new_revision,
                "conflict": conflict,
            }),
        ));

        self.notify(MeshNotification::single(
            artifact_id,
            MeshNotificationKind::Patch,
            dimension_id,
            task_id,
            Some(node_id),
            Vec::new(),
            "application/json".to_owned(),
        ));
    }

    fn insert_replicated_artifact(&self, artifact: MeshArtifact) -> Result<(), MeshError> {
        let id = artifact.id;
        let mut arts = self.artifacts.lock().expect("mesh lock poisoned");
        if arts.contains_key(&id) {
            return Err(MeshError::AlreadyExists(id));
        }
        let dimension_id = artifact.dimension_id;
        let task_id = artifact.task_id;
        let node_id = artifact.node_id;
        let tags = artifact.tags.clone();
        let content_type = artifact.content_type.clone();
        arts.insert(id, artifact.clone());
        drop(arts);
        self.index
            .lock()
            .expect("mesh index lock poisoned")
            .insert(&artifact);
        if let Some(node_id) = node_id {
            self.touch_node_state(node_id, |state| {
                state.last_artifact_id = Some(id);
            });
        }
        self.action_log.append(ActionLogEntry::new(
            EventType::ArtifactProduced,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "artifact_id": id.to_string(),
                "replicated": true,
            }),
        ));
        self.notify(MeshNotification::single(
            id,
            MeshNotificationKind::Replicated,
            dimension_id,
            task_id,
            node_id,
            tags,
            content_type,
        ));
        Ok(())
    }

    fn insert_replicated_snapshot(&self, snapshot: NodeSnapshot) -> Result<(), MeshError> {
        let id = snapshot.artifact_id;
        let mut snaps = self.snapshots.lock().expect("mesh snapshot lock poisoned");
        if snaps.contains_key(&id) {
            return Err(MeshError::AlreadyExists(id));
        }
        let dimension_id = snapshot.dimension_id;
        let task_id = snapshot.task_id;
        let node_id = snapshot.node_id;
        let revision = snapshot.revision;
        snaps.insert(id, snapshot);
        drop(snaps);
        self.touch_node_state(node_id, |state| {
            if revision >= state.revision {
                state.last_snapshot_id = Some(id);
                state.revision = revision;
            }
        });
        self.notify(MeshNotification::single(
            id,
            MeshNotificationKind::Replicated,
            dimension_id,
            task_id,
            Some(node_id),
            Vec::new(),
            "application/json".to_owned(),
        ));
        Ok(())
    }

    fn insert_replicated_patch(&self, patch: DiffPatch) -> Result<(), MeshError> {
        let id = patch.artifact_id;
        let mut patches = self.patches.lock().expect("mesh patch lock poisoned");
        if patches.contains_key(&id) {
            return Err(MeshError::AlreadyExists(id));
        }
        let dimension_id = patch.dimension_id;
        let task_id = patch.task_id;
        let node_id = patch.node_id;
        let revision = patch.new_revision;
        patches.insert(id, patch);
        drop(patches);
        self.touch_node_state(node_id, |state| {
            if revision >= state.revision {
                state.last_patch_id = Some(id);
                state.revision = revision;
            }
        });
        self.notify(MeshNotification::single(
            id,
            MeshNotificationKind::Replicated,
            dimension_id,
            task_id,
            Some(node_id),
            Vec::new(),
            "application/json".to_owned(),
        ));
        Ok(())
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
                tags: Vec::new(),
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
        self.artifact.provenance.producing_node_id = Some(id);
        self
    }

    /// Set the content type.
    pub fn content_type(mut self, ct: impl Into<String>) -> Self {
        self.artifact.content_type = ct.into();
        self
    }

    /// Add a tag for indexing.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.artifact.tags.push(tag.into());
        self
    }

    /// Replace all tags for indexing.
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.artifact.tags = tags;
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

    /// Set the producing agent ID for provenance.
    pub fn agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.artifact.provenance.producing_agent_id = Some(agent_id.into());
        self
    }

    /// Set the controller ID for provenance.
    pub fn controller_id(mut self, controller_id: Uuid) -> Self {
        self.artifact.provenance.controller_id = Some(controller_id);
        self
    }

    /// Set the schema version for provenance.
    pub fn schema_version(mut self, version: impl Into<String>) -> Self {
        self.artifact.provenance.schema_version = version.into();
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
    use std::sync::Arc;

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
        let node_id = Uuid::new_v4();

        let before = serde_json::json!({ "color": "red" });
        let after = serde_json::json!({ "color": "blue" });
        let ops = vec![PatchOp::Replace {
            path: "/color".to_owned(),
            old: serde_json::json!("red"),
            new: serde_json::json!("blue"),
        }];

        let patch_id = store.patch(node_id, before.clone(), after.clone(), ops, task, dim);
        let patch = store.get_patch(patch_id).unwrap();

        assert_eq!(patch.node_id, node_id);
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

    #[test]
    fn schema_registry_validates_versions() {
        let store = make_store();
        store
            .register_schema("text/plain", "2.0.0", "Plain text payloads")
            .unwrap();
        assert!(store.validate_schema("text/plain", "2.0.0").is_ok());
        let err = store.validate_schema("text/plain", "2.1.0");
        assert!(matches!(err, Err(MeshError::SchemaVersionMismatch { .. })));
    }

    #[test]
    fn produce_validated_rejects_unknown_schema() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let artifact = MeshArtifactBuilder::new(dim, task)
            .content_type("text/custom")
            .schema_version("1.2.3")
            .payload(serde_json::json!({"value": 7}))
            .build();
        let err = store.produce_validated(artifact);
        assert!(matches!(err, Err(MeshError::SchemaNotRegistered { .. })));

        store
            .register_schema("text/custom", "1.2.3", "Custom payload")
            .unwrap();
        let artifact = MeshArtifactBuilder::new(dim, task)
            .content_type("text/custom")
            .schema_version("1.2.3")
            .payload(serde_json::json!({"value": 7}))
            .build();
        assert!(store.produce_validated(artifact).is_ok());
    }

    #[test]
    fn produce_batch_indexes_by_tag_and_node() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let node_id = Uuid::new_v4();
        let artifact_a = MeshArtifactBuilder::new(dim, task)
            .node_id(node_id)
            .tag("batch")
            .payload(serde_json::json!({"a": 1}))
            .build();
        let artifact_b = MeshArtifactBuilder::new(dim, task)
            .node_id(node_id)
            .tag("batch")
            .payload(serde_json::json!({"b": 2}))
            .build();

        let ids = store.produce_batch(vec![artifact_a, artifact_b]).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(store.artifact_count(), 2);
        assert_eq!(store.artifacts_for_node(node_id).len(), 2);
        assert_eq!(store.artifacts_with_tag("batch").len(), 2);
    }

    #[test]
    fn patch_with_revision_detects_conflict() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let node_id = Uuid::new_v4();
        let before = serde_json::json!({ "x": 1 });
        let after = serde_json::json!({ "x": 2 });
        let ops = vec![PatchOp::Replace {
            path: "/x".to_owned(),
            old: serde_json::json!(1),
            new: serde_json::json!(2),
        }];

        let result = store
            .patch_with_revision(
                node_id,
                before.clone(),
                after.clone(),
                ops.clone(),
                task,
                dim,
                0,
                ConflictStrategy::Reject,
            )
            .unwrap();
        assert_eq!(result.new_revision, 1);

        let err = store.patch_with_revision(
            node_id,
            before.clone(),
            after.clone(),
            ops.clone(),
            task,
            dim,
            0,
            ConflictStrategy::Reject,
        );
        assert!(matches!(err, Err(MeshError::ConcurrentEdit { .. })));

        let ok = store
            .patch_with_revision(
                node_id,
                before,
                after,
                ops,
                task,
                dim,
                0,
                ConflictStrategy::LastWriteWins,
            )
            .unwrap();
        assert!(ok.conflict);
    }

    #[tokio::test]
    async fn filtered_subscription_matches_tag() {
        let store = make_store();
        let filter = SubscriptionFilter {
            tag: Some("notify".to_owned()),
            ..SubscriptionFilter::default()
        };
        let mut subscription = store.subscribe_filtered(filter);
        let dim = DimensionId::new();
        let task = TaskId::new();

        let tagged = MeshArtifactBuilder::new(dim, task)
            .tag("notify")
            .payload(serde_json::json!({"value": 1}))
            .build();
        let _ = store.produce(tagged);

        let note = subscription.recv().await.unwrap();
        assert_eq!(note.tags, vec!["notify"]);
    }

    #[test]
    fn garbage_collection_removes_ephemeral_consumed() {
        let store = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let artifact = MeshArtifactBuilder::new(dim, task)
            .immutability_tier(0)
            .payload(serde_json::json!({"temp": true}))
            .build();
        let id = store.produce(artifact);
        store.consume(id).unwrap();
        let report = store.collect_garbage(&GarbageCollectionPolicy::default());
        assert_eq!(report.artifacts_removed, 1);
        assert_eq!(store.artifact_count(), 0);
    }

    #[test]
    fn replication_copies_artifact() {
        let source = make_store();
        let target = make_store();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let id = source.produce(make_artifact(dim, task));
        let replicator = MeshReplicator::new(Arc::clone(&source), Arc::clone(&target));
        replicator.replicate_artifact(id).unwrap();
        assert_eq!(target.artifact_count(), 1);
    }
}
