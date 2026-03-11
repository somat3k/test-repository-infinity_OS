//! Policy engine for allow/deny decisions — Epic O item 9.
//!
//! [`PolicyEngine`] evaluates a prioritised list of [`PolicyRule`]s against
//! a [`PolicyRequest`] and returns a [`Decision`].  Rules are evaluated in
//! priority order (ascending — lower numbers first); the first matching rule
//! wins.  When multiple rules share the same priority, their relative
//! evaluation order is not guaranteed and must not be relied upon.  If no
//! rule matches, the default decision is [`Decision::Deny`].
//!
//! Rules match on principal ID, action type, resource kind, and optional
//! dimension scope.  Conditions use a conjunction (all conditions must hold).

use std::collections::HashMap;
use std::sync::Arc;

use ify_controller::action_log::{ActionLog, ActionLogEntry, Actor, EventType};
use ify_core::DimensionId;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the policy engine.
#[derive(Debug, Error)]
pub enum PolicyError {
    /// A rule with this ID is already registered.
    #[error("policy rule '{0}' already registered")]
    Duplicate(String),
    /// A rule with this ID was not found.
    #[error("policy rule '{0}' not found")]
    NotFound(String),
}

// ---------------------------------------------------------------------------
// Decision
// ---------------------------------------------------------------------------

/// The outcome of a policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// The request is permitted.
    Allow,
    /// The request is denied.
    Deny,
}

// ---------------------------------------------------------------------------
// ActionType
// ---------------------------------------------------------------------------

/// Identifies the type of action in a policy request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// Reading a resource.
    Read,
    /// Writing a resource.
    Write,
    /// Deleting a resource.
    Delete,
    /// Executing / invoking a resource.
    Execute,
    /// Deploying a workload.
    Deploy,
    /// Publishing to the marketplace.
    Publish,
    /// Administrative action.
    Admin,
    /// Custom action identified by a string.
    Custom(String),
}

// ---------------------------------------------------------------------------
// PolicyRequest
// ---------------------------------------------------------------------------

/// Describes the access being requested, evaluated by the policy engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRequest {
    /// Identifier of the requesting principal.
    pub principal_id: String,
    /// Kind of principal (user / agent / tool).
    pub principal_kind: String,
    /// Action being requested.
    pub action: ActionType,
    /// Resource being accessed (a string identifier).
    pub resource: String,
    /// Dimension scope, if applicable.
    pub dimension_id: Option<DimensionId>,
    /// Additional key-value context (e.g. `{"environment": "production"}`).
    pub context: HashMap<String, String>,
}

impl PolicyRequest {
    /// Construct a minimal request.
    pub fn new(
        principal_id: impl Into<String>,
        principal_kind: impl Into<String>,
        action: ActionType,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            principal_kind: principal_kind.into(),
            action,
            resource: resource.into(),
            dimension_id: None,
            context: HashMap::new(),
        }
    }

    /// Attach dimension scope.
    pub fn with_dimension(mut self, dim: DimensionId) -> Self {
        self.dimension_id = Some(dim);
        self
    }

    /// Attach a context entry.
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

// ---------------------------------------------------------------------------
// PolicyCondition — individual match predicate
// ---------------------------------------------------------------------------

/// A predicate that must match for a rule to fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum PolicyCondition {
    /// Match a specific principal ID.
    PrincipalIs(String),
    /// Match a principal kind (e.g. `"agent"`).
    PrincipalKindIs(String),
    /// Match a specific action.
    ActionIs(ActionType),
    /// Match a resource prefix.
    ResourceStartsWith(String),
    /// Match a specific resource exactly.
    ResourceIs(String),
    /// Require a context key to have a specific value.
    ContextEquals {
        /// Context key to match.
        key: String,
        /// Expected context value.
        value: String,
    },
}

impl PolicyCondition {
    fn matches(&self, req: &PolicyRequest) -> bool {
        match self {
            Self::PrincipalIs(id) => &req.principal_id == id,
            Self::PrincipalKindIs(kind) => &req.principal_kind == kind,
            Self::ActionIs(action) => &req.action == action,
            Self::ResourceStartsWith(prefix) => req.resource.starts_with(prefix.as_str()),
            Self::ResourceIs(r) => &req.resource == r,
            Self::ContextEquals { key, value } => {
                req.context.get(key).map(|v| v == value).unwrap_or(false)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PolicyRule
// ---------------------------------------------------------------------------

/// A named rule that fires when all its conditions match.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Unique rule identifier (e.g. `"allow-agent-read-artifacts"`).
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Conditions (all must match — conjunction).
    pub conditions: Vec<PolicyCondition>,
    /// Decision to apply when the rule fires.
    pub decision: Decision,
    /// Lower numbers are evaluated first.
    pub priority: u32,
}

impl PolicyRule {
    /// Create a rule with a single condition.
    pub fn new(
        id: impl Into<String>,
        description: impl Into<String>,
        decision: Decision,
        priority: u32,
    ) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            conditions: Vec::new(),
            decision,
            priority,
        }
    }

    /// Add a condition.
    pub fn with_condition(mut self, cond: PolicyCondition) -> Self {
        self.conditions.push(cond);
        self
    }

    /// Returns `true` when all conditions match `req`.
    pub fn matches(&self, req: &PolicyRequest) -> bool {
        self.conditions.iter().all(|c| c.matches(req))
    }
}

// ---------------------------------------------------------------------------
// PolicyEngine
// ---------------------------------------------------------------------------

/// Evaluates [`PolicyRule`]s against [`PolicyRequest`]s.
///
/// Rules are stored in a flat list sorted by priority at insertion time.
/// The first matching rule determines the decision.  If no rule matches,
/// [`Decision::Deny`] is returned (default-deny posture).
///
/// When an [`ActionLog`] is attached via [`PolicyEngine::with_action_log`],
/// every deny decision (whether from an explicit deny rule or the default)
/// emits a [`EventType::SecurityAccessDenied`] entry.
pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
    action_log: Option<Arc<ActionLog>>,
}

impl PolicyEngine {
    /// Create an engine with no rules (default-deny) and no ActionLog.
    pub fn new() -> Self {
        Self { rules: Vec::new(), action_log: None }
    }

    /// Attach an [`ActionLog`] so that deny decisions emit
    /// [`EventType::SecurityAccessDenied`] entries.
    pub fn with_action_log(mut self, log: Arc<ActionLog>) -> Self {
        self.action_log = Some(log);
        self
    }

    /// Add a rule.  Rules are kept sorted by `priority` (ascending).
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::Duplicate`] when a rule with the same ID
    /// already exists.
    pub fn add_rule(&mut self, rule: PolicyRule) -> Result<(), PolicyError> {
        if self.rules.iter().any(|r| r.id == rule.id) {
            return Err(PolicyError::Duplicate(rule.id));
        }
        self.rules.push(rule);
        self.rules.sort_by_key(|r| r.priority);
        Ok(())
    }

    /// Remove a rule by ID.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::NotFound`] when no rule with that ID exists.
    pub fn remove_rule(&mut self, id: &str) -> Result<(), PolicyError> {
        let pos = self.rules.iter().position(|r| r.id == id)
            .ok_or_else(|| PolicyError::NotFound(id.to_owned()))?;
        self.rules.remove(pos);
        Ok(())
    }

    /// Evaluate `request` against all rules and return the decision.
    ///
    /// Returns [`Decision::Deny`] if no rule matches.  If an ActionLog is
    /// attached, a [`EventType::SecurityAccessDenied`] entry is emitted for
    /// every deny outcome (explicit deny rule or default-deny).
    pub fn evaluate(&self, request: &PolicyRequest) -> Decision {
        for rule in &self.rules {
            if rule.matches(request) {
                debug!(
                    rule = %rule.id,
                    decision = ?rule.decision,
                    principal = %request.principal_id,
                    action = ?request.action,
                    resource = %request.resource,
                    "policy rule matched"
                );
                if rule.decision == Decision::Deny {
                    self.emit_access_denied(request, Some(&rule.id));
                }
                return rule.decision;
            }
        }
        warn!(
            principal = %request.principal_id,
            action = ?request.action,
            resource = %request.resource,
            "no policy rule matched; default deny"
        );
        self.emit_access_denied(request, None);
        Decision::Deny
    }

    fn emit_access_denied(&self, request: &PolicyRequest, rule_id: Option<&str>) {
        if let Some(log) = &self.action_log {
            let entry = ActionLogEntry::new(
                EventType::SecurityAccessDenied,
                Actor::System,
                request.dimension_id,
                None,
                serde_json::json!({
                    "principal_id": request.principal_id,
                    "principal_kind": request.principal_kind,
                    "action": format!("{:?}", request.action),
                    "resource": request.resource,
                    "rule_id": rule_id,
                }),
            );
            log.append(entry);
        }
    }

    /// Number of registered rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Return all rules (sorted by priority).
    pub fn rules(&self) -> &[PolicyRule] {
        &self.rules
    }
}

impl Default for PolicyEngine {
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

    fn allow_agents_read() -> PolicyRule {
        PolicyRule::new("allow-agent-read", "Agents may read artifacts", Decision::Allow, 10)
            .with_condition(PolicyCondition::PrincipalKindIs("agent".into()))
            .with_condition(PolicyCondition::ActionIs(ActionType::Read))
    }

    fn deny_tool_write() -> PolicyRule {
        PolicyRule::new("deny-tool-write", "Tools may not write", Decision::Deny, 5)
            .with_condition(PolicyCondition::PrincipalKindIs("tool".into()))
            .with_condition(PolicyCondition::ActionIs(ActionType::Write))
    }

    #[test]
    fn no_rules_default_deny() {
        let engine = PolicyEngine::new();
        let req = PolicyRequest::new("u1", "user", ActionType::Read, "artifact-1");
        assert_eq!(engine.evaluate(&req), Decision::Deny);
    }

    #[test]
    fn matching_rule_allows() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(allow_agents_read()).unwrap();
        let req = PolicyRequest::new("agent-1", "agent", ActionType::Read, "artifact-1");
        assert_eq!(engine.evaluate(&req), Decision::Allow);
    }

    #[test]
    fn matching_rule_denies() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(deny_tool_write()).unwrap();
        let req = PolicyRequest::new("tool-1", "tool", ActionType::Write, "artifact-1");
        assert_eq!(engine.evaluate(&req), Decision::Deny);
    }

    #[test]
    fn priority_order_respected() {
        let mut engine = PolicyEngine::new();
        // Priority 5 deny fires before priority 10 allow.
        engine.add_rule(allow_agents_read()).unwrap(); // prio 10
        engine.add_rule(
            PolicyRule::new("deny-all-read", "Deny all reads", Decision::Deny, 1)
                .with_condition(PolicyCondition::ActionIs(ActionType::Read)),
        ).unwrap();

        let req = PolicyRequest::new("agent-1", "agent", ActionType::Read, "artifact-1");
        assert_eq!(engine.evaluate(&req), Decision::Deny);
    }

    #[test]
    fn duplicate_rule_rejected() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(allow_agents_read()).unwrap();
        assert!(matches!(
            engine.add_rule(allow_agents_read()),
            Err(PolicyError::Duplicate(_))
        ));
    }

    #[test]
    fn remove_rule() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(allow_agents_read()).unwrap();
        assert_eq!(engine.rule_count(), 1);
        engine.remove_rule("allow-agent-read").unwrap();
        assert_eq!(engine.rule_count(), 0);
    }

    #[test]
    fn resource_prefix_condition() {
        let mut engine = PolicyEngine::new();
        engine
            .add_rule(
                PolicyRule::new("allow-public", "Allow public resources", Decision::Allow, 1)
                    .with_condition(PolicyCondition::ResourceStartsWith("public/".into())),
            )
            .unwrap();

        let req = PolicyRequest::new("u1", "user", ActionType::Read, "public/data.json");
        assert_eq!(engine.evaluate(&req), Decision::Allow);

        let req2 = PolicyRequest::new("u1", "user", ActionType::Read, "private/data.json");
        assert_eq!(engine.evaluate(&req2), Decision::Deny);
    }

    #[test]
    fn context_condition() {
        let mut engine = PolicyEngine::new();
        engine
            .add_rule(
                PolicyRule::new("prod-only", "Deny in production", Decision::Deny, 1)
                    .with_condition(PolicyCondition::ContextEquals {
                        key: "environment".into(),
                        value: "production".into(),
                    }),
            )
            .unwrap();

        let prod_req = PolicyRequest::new("u1", "user", ActionType::Write, "db")
            .with_context("environment", "production");
        assert_eq!(engine.evaluate(&prod_req), Decision::Deny);

        let dev_req = PolicyRequest::new("u1", "user", ActionType::Write, "db")
            .with_context("environment", "development");
        assert_eq!(engine.evaluate(&dev_req), Decision::Deny); // no allow rule
    }

    #[test]
    fn deny_emits_action_log_event() {
        use ify_controller::action_log::{ActionLog, EventType};
        let log = ActionLog::new(16);
        let mut rx = log.subscribe();
        let mut engine = PolicyEngine::new().with_action_log(log);
        engine.add_rule(allow_agents_read()).unwrap();

        // A non-agent principal requesting read should default-deny and emit.
        let req = PolicyRequest::new("user-1", "user", ActionType::Read, "artifact-1");
        assert_eq!(engine.evaluate(&req), Decision::Deny);

        let entry = rx.try_recv().expect("ActionLog entry must be emitted on deny");
        assert_eq!(entry.event_type, EventType::SecurityAccessDenied);
    }
}
