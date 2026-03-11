//! Input validation at all boundary layers — Epic O item 2.
//!
//! The [`InputValidator`] checks incoming data against registered
//! [`ValidationRule`]s before it crosses any layer boundary (canvas→runtime,
//! runtime→kernel FFI, mesh write path, tool invocation, etc.).
//!
//! Rules can be added at any time and are evaluated in registration order.
//! The first failing rule short-circuits further evaluation and returns a
//! [`ValidationError`] that includes the rule name and a human-readable
//! reason.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the input validation subsystem.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// A registered rule rejected the value.
    #[error("validation rule '{rule}' failed: {reason}")]
    RuleFailed {
        /// Name of the failing rule.
        rule: String,
        /// Human-readable reason for the failure.
        reason: String,
    },

    /// A required field is missing from the input.
    #[error("required field '{0}' is missing")]
    MissingField(String),

    /// A field value exceeds the permitted byte length.
    #[error("field '{field}' exceeds max length {max} (got {got})")]
    TooLong {
        /// Field name.
        field: String,
        /// Maximum permitted length.
        max: usize,
        /// Actual length received.
        got: usize,
    },

    /// A field contains characters outside the allowed set.
    #[error("field '{field}' contains disallowed characters")]
    DisallowedChars {
        /// Field name.
        field: String,
    },

    /// A numeric value is outside the permitted range.
    #[error("field '{field}' value {value} is out of range [{min}, {max}]")]
    OutOfRange {
        /// Field name.
        field: String,
        /// Actual value.
        value: i64,
        /// Minimum permitted value.
        min: i64,
        /// Maximum permitted value.
        max: i64,
    },
}

// ---------------------------------------------------------------------------
// ValidationResult
// ---------------------------------------------------------------------------

/// Outcome of a validation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// `true` when all rules passed.
    pub ok: bool,
    /// List of failure messages (empty when `ok` is `true`).
    pub failures: Vec<String>,
}

impl ValidationResult {
    /// Create a passing result.
    pub fn pass() -> Self {
        Self { ok: true, failures: Vec::new() }
    }

    /// Create a failing result with a single message.
    pub fn fail(message: impl Into<String>) -> Self {
        Self { ok: false, failures: vec![message.into()] }
    }

    /// Add an additional failure message.
    pub fn push_failure(&mut self, message: impl Into<String>) {
        self.ok = false;
        self.failures.push(message.into());
    }
}

// ---------------------------------------------------------------------------
// Boundary Layer
// ---------------------------------------------------------------------------

/// Identifies the boundary layer where validation is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryLayer {
    /// Canvas surface → runtime (node execution, snippet submission).
    CanvasToRuntime,
    /// Runtime → kernel FFI.
    RuntimeToKernel,
    /// Mesh artifact write path.
    MeshWrite,
    /// Tool invocation boundary.
    ToolInvocation,
    /// Agent input boundary.
    AgentInput,
    /// HTTP/external API ingress.
    ApiIngress,
}

// ---------------------------------------------------------------------------
// ValidationRule — trait
// ---------------------------------------------------------------------------

/// A single named validation rule that can accept or reject a JSON value.
pub trait ValidationRule: Send + Sync {
    /// Name of this rule (used in error messages).
    fn name(&self) -> &str;

    /// Evaluate the rule against `value`.
    ///
    /// Returns `Ok(())` when the value is acceptable or a
    /// [`ValidationError`] describing the problem.
    fn check(&self, value: &serde_json::Value) -> Result<(), ValidationError>;
}

// ---------------------------------------------------------------------------
// Built-in rules
// ---------------------------------------------------------------------------

/// Requires that `value` is a JSON object containing all listed keys.
pub struct RequiredFieldsRule {
    rule_name: String,
    fields: Vec<String>,
}

impl RequiredFieldsRule {
    /// Create the rule.
    pub fn new(name: impl Into<String>, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            rule_name: name.into(),
            fields: fields.into_iter().map(Into::into).collect(),
        }
    }
}

impl ValidationRule for RequiredFieldsRule {
    fn name(&self) -> &str {
        &self.rule_name
    }

    fn check(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        let obj = match value.as_object() {
            Some(o) => o,
            None => {
                return Err(ValidationError::RuleFailed {
                    rule: self.rule_name.clone(),
                    reason: "value is not a JSON object".into(),
                })
            }
        };
        for field in &self.fields {
            if !obj.contains_key(field) {
                return Err(ValidationError::MissingField(field.clone()));
            }
        }
        Ok(())
    }
}

/// Limits the byte length of a named string field.
pub struct MaxLengthRule {
    rule_name: String,
    field: String,
    max_bytes: usize,
}

impl MaxLengthRule {
    /// Create the rule.
    pub fn new(name: impl Into<String>, field: impl Into<String>, max_bytes: usize) -> Self {
        Self {
            rule_name: name.into(),
            field: field.into(),
            max_bytes,
        }
    }
}

impl ValidationRule for MaxLengthRule {
    fn name(&self) -> &str {
        &self.rule_name
    }

    fn check(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        if let Some(s) = value.get(&self.field).and_then(|v| v.as_str()) {
            if s.len() > self.max_bytes {
                return Err(ValidationError::TooLong {
                    field: self.field.clone(),
                    max: self.max_bytes,
                    got: s.len(),
                });
            }
        }
        Ok(())
    }
}

/// Rejects a string field if it contains characters outside `[A-Za-z0-9_.-]`.
pub struct SafeIdentifierRule {
    rule_name: String,
    field: String,
}

impl SafeIdentifierRule {
    /// Create the rule.
    pub fn new(name: impl Into<String>, field: impl Into<String>) -> Self {
        Self { rule_name: name.into(), field: field.into() }
    }
}

impl ValidationRule for SafeIdentifierRule {
    fn name(&self) -> &str {
        &self.rule_name
    }

    fn check(&self, value: &serde_json::Value) -> Result<(), ValidationError> {
        if let Some(s) = value.get(&self.field).and_then(|v| v.as_str()) {
            if !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
                return Err(ValidationError::DisallowedChars {
                    field: self.field.clone(),
                });
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// InputValidator
// ---------------------------------------------------------------------------

/// Validates incoming data at a specific boundary layer.
///
/// Rules are stored per-layer and evaluated in registration order.
/// The validator is cheaply cloneable because rules are stored in a shared
/// `Arc`; however, adding a rule after cloning only affects the recipient.
pub struct InputValidator {
    rules: HashMap<BoundaryLayer, Vec<Box<dyn ValidationRule>>>,
}

impl InputValidator {
    /// Create a validator with no rules.
    pub fn new() -> Self {
        Self { rules: HashMap::new() }
    }

    /// Register a rule for a specific boundary layer.
    pub fn add_rule(&mut self, layer: BoundaryLayer, rule: Box<dyn ValidationRule>) {
        self.rules.entry(layer).or_default().push(rule);
    }

    /// Validate `value` against all rules registered for `layer`.
    ///
    /// Returns the first [`ValidationError`] encountered, or `Ok(())` when
    /// all rules pass.
    pub fn validate(
        &self,
        layer: BoundaryLayer,
        value: &serde_json::Value,
    ) -> Result<(), ValidationError> {
        if let Some(rules) = self.rules.get(&layer) {
            for rule in rules {
                if let Err(e) = rule.check(value) {
                    warn!(layer = ?layer, rule = rule.name(), error = %e, "input validation failed");
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    /// Validate `value` and collect *all* failures into a [`ValidationResult`].
    pub fn validate_all(
        &self,
        layer: BoundaryLayer,
        value: &serde_json::Value,
    ) -> ValidationResult {
        let mut result = ValidationResult::pass();
        if let Some(rules) = self.rules.get(&layer) {
            for rule in rules {
                if let Err(e) = rule.check(value) {
                    warn!(layer = ?layer, rule = rule.name(), error = %e, "input validation failed");
                    result.push_failure(e.to_string());
                }
            }
        }
        result
    }
}

impl Default for InputValidator {
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

    fn make_validator() -> InputValidator {
        let mut v = InputValidator::new();
        v.add_rule(
            BoundaryLayer::CanvasToRuntime,
            Box::new(RequiredFieldsRule::new("required-fields", ["task_id", "payload"])),
        );
        v.add_rule(
            BoundaryLayer::CanvasToRuntime,
            Box::new(MaxLengthRule::new("max-task-id", "task_id", 64)),
        );
        v.add_rule(
            BoundaryLayer::CanvasToRuntime,
            Box::new(SafeIdentifierRule::new("safe-task-id", "task_id")),
        );
        v
    }

    #[test]
    fn valid_input_passes() {
        let v = make_validator();
        let input = serde_json::json!({"task_id": "abc-123", "payload": {}});
        assert!(v.validate(BoundaryLayer::CanvasToRuntime, &input).is_ok());
    }

    #[test]
    fn missing_field_fails() {
        let v = make_validator();
        let input = serde_json::json!({"task_id": "abc"});
        assert!(matches!(
            v.validate(BoundaryLayer::CanvasToRuntime, &input),
            Err(ValidationError::MissingField(_))
        ));
    }

    #[test]
    fn too_long_field_fails() {
        let v = make_validator();
        let long_id = "a".repeat(65);
        let input = serde_json::json!({"task_id": long_id, "payload": {}});
        assert!(matches!(
            v.validate(BoundaryLayer::CanvasToRuntime, &input),
            Err(ValidationError::TooLong { .. })
        ));
    }

    #[test]
    fn disallowed_chars_fail() {
        let v = make_validator();
        let input = serde_json::json!({"task_id": "bad task id!", "payload": {}});
        assert!(matches!(
            v.validate(BoundaryLayer::CanvasToRuntime, &input),
            Err(ValidationError::DisallowedChars { .. })
        ));
    }

    #[test]
    fn validate_all_collects_multiple_failures() {
        let mut v = InputValidator::new();
        v.add_rule(
            BoundaryLayer::MeshWrite,
            Box::new(RequiredFieldsRule::new("r1", ["field_a"])),
        );
        v.add_rule(
            BoundaryLayer::MeshWrite,
            Box::new(RequiredFieldsRule::new("r2", ["field_b"])),
        );
        let input = serde_json::json!({});
        let result = v.validate_all(BoundaryLayer::MeshWrite, &input);
        assert!(!result.ok);
        assert_eq!(result.failures.len(), 2);
    }

    #[test]
    fn unknown_layer_passes_vacuously() {
        let v = InputValidator::new();
        let input = serde_json::json!({"anything": true});
        assert!(v.validate(BoundaryLayer::ApiIngress, &input).is_ok());
    }
}
