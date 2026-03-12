//! Identity-first access controls — Epic O item 4.
//!
//! Provides [`Principal`] (user, agent, tool), [`IdentityRegistry`], and
//! [`AccessPolicy`] types that together implement a least-privilege,
//! identity-first access control model.
//!
//! Every resource access must be approved by `AccessPolicy::check`, which
//! evaluates whether the requesting principal holds the required
//! [`Capabilities`] for the requested [`ResourceKind`].

use std::collections::HashMap;

use ify_core::{Capabilities, DimensionId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the identity subsystem.
#[derive(Debug, Error)]
pub enum IdentityError {
    /// The principal is not registered.
    #[error("principal '{0}' not found")]
    NotFound(String),
    /// The principal is already registered.
    #[error("principal '{0}' already registered")]
    Duplicate(String),
    /// Access was denied.
    #[error("access denied for principal '{principal}': missing capability {missing:?}")]
    AccessDenied {
        /// Principal that was denied.
        principal: String,
        /// Capability that was required but not held.
        missing: Capabilities,
    },
}

// ---------------------------------------------------------------------------
// PrincipalKind
// ---------------------------------------------------------------------------

/// Identifies the type of principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    /// A human user.
    User,
    /// An autonomous agent.
    Agent,
    /// An external tool (DB, HTTP, blockchain, model).
    Tool,
}

// ---------------------------------------------------------------------------
// Principal
// ---------------------------------------------------------------------------

/// An identity that can request access to resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    /// Unique identifier for this principal.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// Kind of principal.
    pub kind: PrincipalKind,
    /// Capabilities granted to this principal.
    pub capabilities: Capabilities,
    /// The dimension this principal is scoped to, if any.
    pub dimension_id: Option<DimensionId>,
}

impl Principal {
    /// Create a new principal.
    pub fn new(
        name: impl Into<String>,
        kind: PrincipalKind,
        capabilities: Capabilities,
        dimension_id: Option<DimensionId>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            capabilities,
            dimension_id,
        }
    }

    /// Returns `true` when this principal holds all bits in `required`.
    pub fn has_capabilities(&self, required: Capabilities) -> bool {
        self.capabilities.contains(required)
    }
}

// ---------------------------------------------------------------------------
// ResourceKind
// ---------------------------------------------------------------------------

/// Classifies the resource being accessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// A secret value.
    Secret,
    /// A mesh artifact.
    Artifact,
    /// A canvas node.
    Node,
    /// A deployment workflow.
    Deployment,
    /// An ML model.
    Model,
    /// A blockchain wallet or transaction.
    Blockchain,
    /// Administrative configuration.
    AdminConfig,
}

impl ResourceKind {
    /// Returns the [`Capabilities`] required to access this resource kind.
    pub fn required_capability(self) -> Capabilities {
        match self {
            Self::Secret => Capabilities::READ_SECRETS,
            Self::Artifact => Capabilities::READ_ARTIFACTS,
            Self::Node => Capabilities::READ_ARTIFACTS,
            Self::Deployment => Capabilities::DEPLOY,
            Self::Model => Capabilities::INVOKE_MODEL,
            Self::Blockchain => Capabilities::INVOKE_TOOLS,
            Self::AdminConfig => Capabilities::ADMIN,
        }
    }
}

// ---------------------------------------------------------------------------
// AccessPolicy
// ---------------------------------------------------------------------------

/// Evaluates whether a principal may access a resource.
#[derive(Debug, Default)]
pub struct AccessPolicy;

impl AccessPolicy {
    /// Create a new policy (stateless; all logic is in `check`).
    pub fn new() -> Self {
        Self
    }

    /// Check whether `principal` may access `resource`.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::AccessDenied`] when the principal lacks the
    /// required capability.
    pub fn check(
        &self,
        principal: &Principal,
        resource: ResourceKind,
    ) -> Result<(), IdentityError> {
        let required = resource.required_capability();
        if principal.has_capabilities(required) {
            Ok(())
        } else {
            warn!(
                principal = %principal.name,
                resource = ?resource,
                missing = ?required,
                "access denied"
            );
            Err(IdentityError::AccessDenied {
                principal: principal.name.clone(),
                missing: required,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// IdentityRegistry
// ---------------------------------------------------------------------------

/// Registry of all known principals within the system.
pub struct IdentityRegistry {
    principals: HashMap<Uuid, Principal>,
}

impl IdentityRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { principals: HashMap::new() }
    }

    /// Register a principal.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::Duplicate`] when a principal with the same
    /// `name` is already registered.
    pub fn register(&mut self, principal: Principal) -> Result<Uuid, IdentityError> {
        let by_name = self
            .principals
            .values()
            .find(|p| p.name == principal.name);
        if by_name.is_some() {
            return Err(IdentityError::Duplicate(principal.name));
        }
        let id = principal.id;
        self.principals.insert(id, principal);
        Ok(id)
    }

    /// Look up a principal by ID.
    pub fn get(&self, id: Uuid) -> Option<&Principal> {
        self.principals.get(&id)
    }

    /// Look up a principal by name.
    pub fn get_by_name(&self, name: &str) -> Option<&Principal> {
        self.principals.values().find(|p| p.name == name)
    }

    /// Remove a principal.
    pub fn deregister(&mut self, id: Uuid) -> Option<Principal> {
        self.principals.remove(&id)
    }

    /// Number of registered principals.
    pub fn len(&self) -> usize {
        self.principals.len()
    }

    /// Returns `true` when no principals are registered.
    pub fn is_empty(&self) -> bool {
        self.principals.is_empty()
    }
}

impl Default for IdentityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_capability_check() {
        let p = Principal::new(
            "agent-deploy",
            PrincipalKind::Agent,
            Capabilities::DEPLOY | Capabilities::READ_ARTIFACTS,
            None,
        );
        assert!(p.has_capabilities(Capabilities::DEPLOY));
        assert!(!p.has_capabilities(Capabilities::ADMIN));
    }

    #[test]
    fn access_policy_allows_sufficient_caps() {
        let policy = AccessPolicy::new();
        let p = Principal::new(
            "agent",
            PrincipalKind::Agent,
            Capabilities::DEPLOY,
            None,
        );
        assert!(policy.check(&p, ResourceKind::Deployment).is_ok());
    }

    #[test]
    fn access_policy_denies_missing_caps() {
        let policy = AccessPolicy::new();
        let p = Principal::new(
            "user",
            PrincipalKind::User,
            Capabilities::READ_ARTIFACTS,
            None,
        );
        assert!(matches!(
            policy.check(&p, ResourceKind::Secret),
            Err(IdentityError::AccessDenied { .. })
        ));
    }

    #[test]
    fn registry_register_and_lookup() {
        let mut reg = IdentityRegistry::new();
        let principal = Principal::new("tool-db", PrincipalKind::Tool, Capabilities::INVOKE_TOOLS, None);
        let id = reg.register(principal).unwrap();
        assert!(reg.get(id).is_some());
        assert_eq!(reg.get_by_name("tool-db").unwrap().kind, PrincipalKind::Tool);
    }

    #[test]
    fn registry_prevents_duplicates() {
        let mut reg = IdentityRegistry::new();
        let p1 = Principal::new("alice", PrincipalKind::User, Capabilities::NONE, None);
        let p2 = Principal::new("alice", PrincipalKind::User, Capabilities::NONE, None);
        reg.register(p1).unwrap();
        assert!(matches!(reg.register(p2), Err(IdentityError::Duplicate(_))));
    }

    #[test]
    fn registry_deregister() {
        let mut reg = IdentityRegistry::new();
        let p = Principal::new("tmp", PrincipalKind::Agent, Capabilities::NONE, None);
        let id = reg.register(p).unwrap();
        assert!(reg.deregister(id).is_some());
        assert!(reg.get(id).is_none());
    }
}
