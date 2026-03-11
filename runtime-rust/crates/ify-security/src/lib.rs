//! # ify-security — Operational Security
//!
//! This crate implements the full **Epic O** feature set for infinityOS.
//! It provides a layered security substrate: threat modelling, input
//! validation, privileged-action audit, identity-first access control,
//! artifact signing, sandboxed tool execution, secret management, supply
//! chain verification, and a policy engine.
//!
//! ## Module map
//!
//! | Module | Epic O item |
//! |--------|-------------|
//! | [`threat_model`] | Threat-model desktop-to-canvas execution path (item 1) |
//! | [`validator`] | Input validation at all boundary layers (item 2) |
//! | [`audit`] | Audit trail for privileged actions (item 3) |
//! | [`identity`] | Identity-first access controls (users/agents/tools) (item 4) |
//! | [`artifact_signing`] | Signed artifacts for runtime/deploy paths (item 5) |
//! | [`sandbox`] | Sandboxed tool execution (network/fs/model boundaries) (item 6) |
//! | [`secrets`] | Secret management and redaction (item 7) |
//! | [`supply_chain`] | Supply chain protections (SBOM, signature verification) (item 8) |
//! | [`policy`] | Policy engine for allow/deny decisions (item 9) |
//!
//! Item 10 (security hardening checklist before GA) is delivered as
//! `docs/governance/security-hardening-checklist.md`.
//!
//! ## Quick start
//!
//! ```rust
//! use std::sync::Arc;
//! use ify_security::{
//!     threat_model::ThreatModel,
//!     validator::{InputValidator, BoundaryLayer, MaxLengthRule},
//!     identity::{Principal, PrincipalKind, IdentityRegistry, AccessPolicy, ResourceKind},
//!     sandbox::{SandboxPolicy, SandboxProfile, SandboxEnforcer, SandboxResource},
//!     secrets::{SecretStore, Redactor},
//!     policy::{PolicyEngine, PolicyRule, PolicyCondition, PolicyRequest, ActionType, Decision},
//! };
//! use ify_controller::action_log::ActionLog;
//! use ify_core::Capabilities;
//!
//! // 1 — Threat model
//! let model = ThreatModel::desktop_to_canvas();
//! assert!(!model.is_empty());
//!
//! // 2 — Input validation
//! let mut validator = InputValidator::new();
//! validator.add_rule(
//!     BoundaryLayer::CanvasToRuntime,
//!     Box::new(MaxLengthRule::new("max-id", "task_id", 64)),
//! );
//! let input = serde_json::json!({"task_id": "abc"});
//! assert!(validator.validate(BoundaryLayer::CanvasToRuntime, &input).is_ok());
//!
//! // 3 — Identity + access control
//! let policy = AccessPolicy::new();
//! let agent = Principal::new("agent-1", PrincipalKind::Agent, Capabilities::DEPLOY, None);
//! assert!(policy.check(&agent, ResourceKind::Deployment).is_ok());
//!
//! // 4 — Sandbox
//! let mut sb_policy = SandboxPolicy::new();
//! sb_policy.register(SandboxProfile::deny_all("my-tool").with_path("/tmp/my-tool"));
//! let enforcer = SandboxEnforcer::new(&sb_policy);
//! assert!(enforcer.check("my-tool", &SandboxResource::Path("/tmp/my-tool/data".into())).is_ok());
//!
//! // 5 — Secrets + redaction
//! let mut store = SecretStore::new();
//! store.register("token", b"secret-value".as_slice()).unwrap();
//! let mut redactor = Redactor::new();
//! redactor.add_literal("token", "secret-value");
//! assert_eq!(redactor.redact("auth=secret-value"), "auth=[REDACTED]");
//!
//! // 6 — Policy engine
//! let mut engine = PolicyEngine::new();
//! engine.add_rule(
//!     PolicyRule::new("allow-agents", "Agents may read", Decision::Allow, 10)
//!         .with_condition(PolicyCondition::PrincipalKindIs("agent".into()))
//!         .with_condition(PolicyCondition::ActionIs(ActionType::Read)),
//! ).unwrap();
//! let req = PolicyRequest::new("agent-1", "agent", ActionType::Read, "artifact-1");
//! assert_eq!(engine.evaluate(&req), Decision::Allow);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod artifact_signing;
pub mod audit;
pub mod identity;
pub mod policy;
pub mod sandbox;
pub mod secrets;
pub mod supply_chain;
pub mod threat_model;
pub mod validator;

// ---------------------------------------------------------------------------
// Crate-level re-exports
// ---------------------------------------------------------------------------

// artifact_signing
pub use artifact_signing::{ArtifactSignature, ArtifactSigner, ArtifactVerifier, SignedArtifact, SigningError};

// audit
pub use audit::{AuditError, AuditRecord, PrivilegedActionKind, PrivilegedAuditLog};

// identity
pub use identity::{AccessPolicy, IdentityError, IdentityRegistry, Principal, PrincipalKind, ResourceKind};

// policy
pub use policy::{
    ActionType, Decision, PolicyCondition, PolicyEngine, PolicyError, PolicyRequest, PolicyRule,
};

// sandbox
pub use sandbox::{
    SandboxEnforcer, SandboxError, SandboxPolicy, SandboxProfile, SandboxResource,
};

// secrets
pub use secrets::{Redactor, RedactionPattern, SecretError, SecretStore, REDACTED_MARKER};

// supply_chain
pub use supply_chain::{
    ComponentKind, ComponentRecord, Sbom, SupplyChainError, SupplyChainVerifier,
};

// threat_model
pub use threat_model::{Mitigation, RiskLevel, ThreatCategory, ThreatEntry, ThreatLayer, ThreatModel};

// validator
pub use validator::{
    BoundaryLayer, InputValidator, MaxLengthRule, RequiredFieldsRule, SafeIdentifierRule,
    ValidationError, ValidationResult,
};
