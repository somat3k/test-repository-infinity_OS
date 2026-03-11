//! Stable **Mesh Artifact API** — read, write, and subscribe across layers.
//!
//! This module defines the [`MeshArtifactApi`] and [`MeshSubscriberApi`] traits
//! that any mesh artifact store in infinityOS must satisfy.
//!
//! ## Stability guarantee
//!
//! Both traits are versioned at
//! [`MESH_ARTIFACT_API_VERSION`](super::versioning::MESH_ARTIFACT_API_VERSION).
//!
//! ## Reference implementation
//!
//! - [`MeshArtifactStore`](ify_controller::mesh::MeshArtifactStore) in
//!   `ify-controller` implements both [`MeshArtifactApi`] and
//!   [`MeshSubscriberApi`].

use ify_core::{ArtifactId, DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Common types
// ---------------------------------------------------------------------------

/// Immutability tier for a mesh artifact.
///
/// | Value | Name | Lifetime |
/// |-------|------|---------|
/// | `0` | Ephemeral | Duration of producing task |
/// | `1` | Session | Duration of owning dimension |
/// | `2` | Persistent | Until explicitly archived or deleted |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ImmutabilityTier {
    /// Exists only for the duration of a task.
    Ephemeral = 0,
    /// Retained for the dimension session lifetime.
    Session = 1,
    /// Retained indefinitely until archived or deleted.
    Persistent = 2,
}

/// Minimal provenance record required by the mesh artifact API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactProvenanceRef {
    /// Task that produced this artifact.
    pub producing_task_id: TaskId,
    /// Canvas node that produced this artifact, if any.
    pub producing_node_id: Option<Uuid>,
    /// Schema version of the artifact content.
    pub schema_version: String,
}

// ---------------------------------------------------------------------------
// MeshArtifactApi
// ---------------------------------------------------------------------------

/// Stable trait for mesh artifact write operations (produce, consume, patch).
///
/// Implementors persist artifacts, enforce immutability semantics, and emit
/// [`ActionLog`](crate::event_bus::EventBusApi) entries for every mutation.
///
/// ## Semver contract
///
/// Versioned at
/// [`MESH_ARTIFACT_API_VERSION`](super::versioning::MESH_ARTIFACT_API_VERSION) `1.0.0`.
pub trait MeshArtifactApi: Send + Sync {
    /// The concrete artifact type stored by this implementation.
    type Artifact: Clone + Send + Sync + 'static;

    /// The concrete snapshot type.
    type Snapshot: Clone + Send + Sync + 'static;

    /// The concrete diff/patch type.
    type Patch: Clone + Send + Sync + 'static;

    /// The error type returned by fallible operations.
    type Error: std::error::Error + Send + Sync + 'static;

    // --- Write operations ---

    /// Commit `artifact` to the store and return its [`ArtifactId`].
    ///
    /// Emits an `artifact.produced` ActionLog entry.
    fn produce(&self, artifact: Self::Artifact) -> ArtifactId;

    /// Consume (read-once) the artifact identified by `id`.
    ///
    /// Returns [`Err`] if the artifact is not found or has already been consumed.
    /// Emits an `artifact.consumed` ActionLog entry.
    fn consume(&self, id: ArtifactId) -> Result<Self::Artifact, Self::Error>;

    // --- Snapshot ---

    /// Capture a point-in-time snapshot of `node_id` and store it.
    ///
    /// Returns the [`ArtifactId`] of the snapshot artifact.
    /// Emits an `artifact.snapshot` ActionLog entry.
    fn snapshot_node(
        &self,
        dimension_id: DimensionId,
        task_id: TaskId,
        node_id: Uuid,
        content: serde_json::Value,
    ) -> ArtifactId;

    /// Retrieve a previously stored snapshot.
    fn get_snapshot(&self, id: ArtifactId) -> Result<Self::Snapshot, Self::Error>;

    // --- Diff / patch ---

    /// Apply and store `ops` as a diff/patch artifact.
    ///
    /// Returns the [`ArtifactId`] of the patch artifact.
    /// Emits an `artifact.patched` ActionLog entry.
    fn patch(
        &self,
        dimension_id: DimensionId,
        task_id: TaskId,
        node_id: Uuid,
        ops: serde_json::Value,
    ) -> ArtifactId;

    /// Retrieve a previously stored patch.
    fn get_patch(&self, id: ArtifactId) -> Result<Self::Patch, Self::Error>;

    // --- Query ---

    /// Number of artifacts currently stored (not counting snapshots/patches).
    fn artifact_count(&self) -> usize;
}

// ---------------------------------------------------------------------------
// MeshSubscriberApi
// ---------------------------------------------------------------------------

/// Stable trait for mesh artifact subscriptions.
///
/// Consumers subscribe to the mesh to receive [`ArtifactId`] notifications
/// whenever new artifacts are produced, consumed, snapshotted, or patched.
///
/// ## Semver contract
///
/// Versioned at
/// [`MESH_ARTIFACT_API_VERSION`](super::versioning::MESH_ARTIFACT_API_VERSION) `1.0.0`.
pub trait MeshSubscriberApi: Send + Sync {
    /// Subscribe to all artifact notifications.
    ///
    /// The returned receiver only sees events published *after* this call.
    fn subscribe(&self) -> broadcast::Receiver<ArtifactId>;
}
