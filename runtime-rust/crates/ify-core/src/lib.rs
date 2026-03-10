//! # ify-core — infinityOS Shared Types
//!
//! This crate is the foundation of the infinityOS Rust workspace.  It defines
//! the core types that every other crate depends on:
//!
//! - [`TaskId`] — globally unique, time-ordered task identifier (UUID v7).
//! - [`DimensionId`] — opaque handle for an isolated execution namespace.
//! - [`ArtifactId`] — opaque handle for a mesh or node artifact.
//! - [`Capabilities`] — bitmask of granted runtime capabilities.
//! - [`IfyError`] — unified error kind for the entire workspace.
//!
//! ## Design Invariants
//!
//! - `TaskId` is **monotonically increasing** within a dimension and
//!   **globally unique** across all dimensions.
//! - `DimensionId` is **opaque**; consumers must not attempt to deserialize
//!   the internal layout.
//! - All public types implement `Send + Sync` to allow safe use across async
//!   task boundaries.
//!
//! See [`docs/architecture/`](../../docs/architecture/) for the full
//! specification of each type.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// TaskId
// ---------------------------------------------------------------------------

/// Globally unique, time-ordered task identifier.
///
/// Internally a UUID v7 (Unix-epoch millisecond timestamp + random bits),
/// which provides both global uniqueness and monotonic ordering within the
/// same millisecond window.
///
/// ## Invariants
///
/// 1. Each `TaskId` is unique across all dimensions.
/// 2. `TaskId`s generated in the same dimension are monotonically increasing.
/// 3. A `TaskId` must never be reused, even after task completion.
///
/// See `docs/architecture/taskid-invariants.md` for the full specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct TaskId(Uuid);

impl TaskId {
    /// Generate a new `TaskId` using UUID v7 (time-ordered).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the inner UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Construct a `TaskId` directly from a [`Uuid`].
    ///
    /// Useful for deterministic derivation (e.g., UUID v5) in the
    /// `blockControllerGenerator` task allocator.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Parse a `TaskId` from a UUID string.
    ///
    /// # Errors
    ///
    /// Returns [`IfyCoreError::InvalidId`] if the string is not a valid UUID.
    pub fn parse_str(s: &str) -> Result<Self, IfyCoreError> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|_| IfyCoreError::InvalidId(s.to_owned()))
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

// ---------------------------------------------------------------------------
// DimensionId
// ---------------------------------------------------------------------------

/// Opaque identifier for an isolated execution namespace (dimension).
///
/// A dimension defines a tenancy boundary: tasks, artifacts, and agents
/// belonging to different dimensions cannot interact unless explicitly bridged
/// through a cross-dimension relay.
///
/// See `docs/architecture/dimension-model.md` for the full specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct DimensionId(Uuid);

impl DimensionId {
    /// Create a new `DimensionId` using UUID v4 (random).
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Return the inner UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Construct a `DimensionId` directly from a [`Uuid`].
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Parse a `DimensionId` from a UUID string.
    ///
    /// # Errors
    ///
    /// Returns [`IfyCoreError::InvalidId`] if the string is not a valid UUID.
    pub fn parse_str(s: &str) -> Result<Self, IfyCoreError> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|_| IfyCoreError::InvalidId(s.to_owned()))
    }
}

impl Default for DimensionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DimensionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

// ---------------------------------------------------------------------------
// ArtifactId
// ---------------------------------------------------------------------------

/// Opaque identifier for a mesh or node artifact.
///
/// Artifacts are the immutable outputs of task execution.  An `ArtifactId`
/// uniquely identifies a specific artifact version within the mesh.
///
/// See `docs/architecture/artifact-model.md` for the full specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct ArtifactId(Uuid);

impl ArtifactId {
    /// Create a new `ArtifactId` using UUID v7 (time-ordered).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Return the inner UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Construct an `ArtifactId` directly from a [`Uuid`].
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Parse an `ArtifactId` from a UUID string.
    ///
    /// # Errors
    ///
    /// Returns [`IfyCoreError::InvalidId`] if the string is not a valid UUID.
    pub fn parse_str(s: &str) -> Result<Self, IfyCoreError> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|_| IfyCoreError::InvalidId(s.to_owned()))
    }
}

impl Default for ArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.hyphenated())
    }
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// Bitmask of runtime capabilities granted to an agent or task.
    ///
    /// Capabilities follow the principle of least privilege: agents declare
    /// the capabilities they require, and the kernel grants only those that
    /// are permitted by the active security policy.
    ///
    /// See `docs/architecture/capability-registry.md` for the full
    /// capability taxonomy.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Capabilities: u64 {
        /// No capabilities (default).
        const NONE      = 0;
        /// Access to the kernel memory subsystem.
        const MEMORY    = 1 << 0;
        /// Access to the task scheduler.
        const SCHEDULER = 1 << 1;
        /// Sandboxed filesystem access.
        const FS        = 1 << 2;
        /// Sandboxed network access.
        const NET       = 1 << 3;
        /// Hardware performance counters.
        const PERF      = 1 << 4;
        /// GPU / accelerator access.
        const GPU       = 1 << 5;
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors produced by `ify-core` operations.
#[derive(Debug, Error)]
pub enum IfyCoreError {
    /// An identifier string could not be parsed.
    #[error("invalid identifier: {0}")]
    InvalidId(String),

    /// A required capability is not available.
    #[error("capability not granted: {0:?}")]
    CapabilityDenied(Capabilities),

    /// An unexpected internal error occurred.
    #[error("internal error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_is_unique() {
        let a = TaskId::new();
        let b = TaskId::new();
        assert_ne!(a, b, "two generated TaskIds must not collide");
    }

    #[test]
    fn task_id_roundtrip() {
        let id = TaskId::new();
        let s = id.to_string();
        let parsed = TaskId::parse_str(&s).expect("parse_str must succeed for a valid UUID");
        assert_eq!(id, parsed);
    }

    #[test]
    fn task_id_monotonic_within_ms() {
        // UUID v7 guarantees ms-level monotonicity; generate a small batch
        // and verify ≥ ordering (they may be equal within the same ms tick).
        let ids: Vec<TaskId> = (0..16).map(|_| TaskId::new()).collect();
        for window in ids.windows(2) {
            assert!(window[0] <= window[1], "TaskIds must be non-decreasing");
        }
    }

    #[test]
    fn dimension_id_is_unique() {
        let a = DimensionId::new();
        let b = DimensionId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn artifact_id_roundtrip() {
        let id = ArtifactId::new();
        let s = id.to_string();
        let parsed = ArtifactId::parse_str(&s).expect("roundtrip must succeed");
        assert_eq!(id, parsed);
    }

    #[test]
    fn capabilities_bitflags() {
        let caps = Capabilities::MEMORY | Capabilities::SCHEDULER;
        assert!(caps.contains(Capabilities::MEMORY));
        assert!(caps.contains(Capabilities::SCHEDULER));
        assert!(!caps.contains(Capabilities::FS));
    }
}
