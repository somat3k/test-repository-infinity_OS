//! API versioning constants and semver utilities.
//!
//! Every stable cross-layer API surface in infinityOS carries an explicit
//! `INTERFACE_VERSION` constant that follows [Semantic Versioning 2.0.0].
//!
//! ## Semver rules
//!
//! | Change | Version bump |
//! |--------|-------------|
//! | Add an optional method with a default impl | PATCH |
//! | Add a new required method (breaking) | MAJOR |
//! | Remove or rename a method | MAJOR |
//! | Change a method signature | MAJOR |
//! | Deprecate a method (still present) | MINOR |
//!
//! See `docs/architecture/deprecation-policy.md` for the full deprecation
//! and migration process.
//!
//! [Semantic Versioning 2.0.0]: https://semver.org/

/// Parsed semantic version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InterfaceVersion {
    /// Major version.  A bump indicates a breaking change.
    pub major: u32,
    /// Minor version.  A bump adds backwards-compatible functionality.
    pub minor: u32,
    /// Patch version.  A bump adds backwards-compatible fixes.
    pub patch: u32,
}

impl InterfaceVersion {
    /// Construct a new version.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// Returns `true` if `other` is backward-compatible with `self`.
    ///
    /// Two versions are compatible when they share the same major version
    /// and `other` is at least as new as `self`.
    pub const fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major && other.minor >= self.minor
    }
}

impl std::fmt::Display for InterfaceVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

// ---------------------------------------------------------------------------
// Published API version constants
// ---------------------------------------------------------------------------

/// Stable version of the [`EventBusApi`](super::event_bus::EventBusApi) interface.
///
/// Bump history:
/// - `1.0.0` — initial stable release (Epic L).
pub const EVENT_BUS_API_VERSION: InterfaceVersion = InterfaceVersion::new(1, 0, 0);

/// Stable version of the [`MeshArtifactApi`](super::mesh::MeshArtifactApi) interface.
///
/// Bump history:
/// - `1.0.0` — initial stable release (Epic L).
pub const MESH_ARTIFACT_API_VERSION: InterfaceVersion = InterfaceVersion::new(1, 0, 0);

/// Stable version of the node execution APIs
/// ([`NodePlannerApi`](super::node_execution::NodePlannerApi),
/// [`NodeExecutorApi`](super::node_execution::NodeExecutorApi),
/// [`NodeReporterApi`](super::node_execution::NodeReporterApi)).
///
/// Bump history:
/// - `1.0.0` — initial stable release (Epic L).
pub const NODE_EXECUTION_API_VERSION: InterfaceVersion = InterfaceVersion::new(1, 0, 0);

/// Stable version of the [`EditorIntegrationApi`](super::editor::EditorIntegrationApi) interface.
///
/// Bump history:
/// - `1.0.0` — initial stable release (Epic L).
pub const EDITOR_INTEGRATION_API_VERSION: InterfaceVersion = InterfaceVersion::new(1, 0, 0);

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_display() {
        let v = InterfaceVersion::new(1, 2, 3);
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn version_compatibility_same_major() {
        let v1 = InterfaceVersion::new(1, 0, 0);
        let v2 = InterfaceVersion::new(1, 1, 0);
        assert!(v1.is_compatible_with(&v2), "1.1 is backward-compat with 1.0");
        assert!(!v2.is_compatible_with(&v1), "1.0 is not compat with 1.1");
    }

    #[test]
    fn version_compatibility_different_major() {
        let v1 = InterfaceVersion::new(1, 0, 0);
        let v2 = InterfaceVersion::new(2, 0, 0);
        assert!(!v1.is_compatible_with(&v2));
        assert!(!v2.is_compatible_with(&v1));
    }

    #[test]
    fn api_version_constants_are_stable() {
        // These values must never be changed without a corresponding semver bump.
        assert_eq!(EVENT_BUS_API_VERSION.major, 1);
        assert_eq!(MESH_ARTIFACT_API_VERSION.major, 1);
        assert_eq!(NODE_EXECUTION_API_VERSION.major, 1);
        assert_eq!(EDITOR_INTEGRATION_API_VERSION.major, 1);
    }
}
