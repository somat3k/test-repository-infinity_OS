//! # ify-interfaces — infinityOS Stable Cross-Layer API Traits
//!
//! This crate defines the **stable trait surface** for every cross-layer API
//! in infinityOS.  All other crates consume these traits rather than concrete
//! types, enabling independent evolution of each layer without breaking
//! callers.
//!
//! ## Modules
//!
//! | Module | Trait(s) | API |
//! |--------|----------|-----|
//! | [`event_bus`] | [`EventBusApi`], [`OrchestratorBusApi`] | ActionLog + orchestration events |
//! | [`mesh`] | [`MeshArtifactApi`], [`MeshSubscriberApi`] | Artifact read/write/subscribe |
//! | [`node_execution`] | [`NodePlannerApi`], [`NodeExecutorApi`], [`NodeReporterApi`] | Node execution lifecycle |
//! | [`editor`] | [`EditorIntegrationApi`] | Interpreter attach, LSP, runtimes |
//! | [`versioning`] | — | Semver constants and compatibility check |
//!
//! ## Semver policy
//!
//! Every trait is pinned to a version constant in [`versioning`].  See
//! `docs/architecture/deprecation-policy.md` for the full deprecation and
//! migration procedure.
//!
//! ## Reference implementations
//!
//! The `ify-controller` crate provides reference implementations for all
//! traits.  See `docs/architecture/layer-interfaces.md` for the complete IDL.
//!
//! ## Conformance tests
//!
//! Run `cargo test -p ify-controller` to execute the API conformance suite,
//! which verifies that `ify-controller`'s concrete types satisfy every trait
//! in this crate.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod editor;
pub mod event_bus;
pub mod mesh;
pub mod node_execution;
pub mod versioning;

// Convenient re-exports at crate root.
pub use editor::{BlockId, EditorIntegrationApi, EditorRef, InterpreterRef, RuntimeHandle};
pub use event_bus::{EventBusApi, OrchestratorBusApi, OrchestratorEventKind};
pub use mesh::{ArtifactProvenanceRef, ImmutabilityTier, MeshArtifactApi, MeshSubscriberApi};
pub use node_execution::{NodeExecutorApi, NodePlannerApi, NodeReporterApi};
pub use versioning::{
    InterfaceVersion, EDITOR_INTEGRATION_API_VERSION, EVENT_BUS_API_VERSION,
    MESH_ARTIFACT_API_VERSION, NODE_EXECUTION_API_VERSION,
};
