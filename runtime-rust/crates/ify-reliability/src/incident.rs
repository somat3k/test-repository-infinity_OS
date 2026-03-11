//! Automated incident management — Epic K item 7.
//!
//! This module provides:
//! - [`TelemetrySignal`] — a structured signal emitted by telemetry hooks.
//! - [`IncidentRule`] — a rule that matches a signal pattern and derives an
//!   incident from it.
//! - [`IncidentPipeline`] — evaluates incoming signals against rules and
//!   creates [`Incident`] records automatically.
//!
//! The pipeline is designed to complement the SLO evaluator and chaos engine:
//! when an SLO breach or chaos fault is detected, a signal can be fed here to
//! open an incident automatically.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

use crate::metrics::IncidentSeverity;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the incident pipeline.
#[derive(Debug, Error)]
pub enum IncidentError {
    /// A rule with the given ID already exists.
    #[error("incident rule '{0}' already registered")]
    DuplicateRule(String),
    /// Incident not found.
    #[error("incident '{0}' not found")]
    NotFound(String),
}

// ---------------------------------------------------------------------------
// TelemetrySignal
// ---------------------------------------------------------------------------

/// A structured signal emitted by a telemetry hook or SLO evaluator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySignal {
    /// Signal type identifier (e.g. `"slo.breach"`, `"chaos.fault_injected"`).
    pub signal_type: String,
    /// Dimension ID this signal belongs to.
    pub dimension_id: String,
    /// Optional task ID correlated with this signal.
    pub task_id: Option<String>,
    /// Structured metadata payload.
    pub payload: serde_json::Value,
    /// Unix timestamp (seconds) when the signal was emitted.
    pub timestamp_secs: u64,
}

impl TelemetrySignal {
    /// Create a new telemetry signal with the current timestamp.
    pub fn new(
        signal_type: impl Into<String>,
        dimension_id: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        let timestamp_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            signal_type: signal_type.into(),
            dimension_id: dimension_id.into(),
            task_id: None,
            payload,
            timestamp_secs,
        }
    }
}

// ---------------------------------------------------------------------------
// IncidentRule
// ---------------------------------------------------------------------------

/// Defines when and how to create an incident from a telemetry signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentRule {
    /// Unique rule ID.
    pub id: String,
    /// Signal type prefix to match (e.g. `"slo."` matches all SLO signals).
    pub signal_type_prefix: String,
    /// Title template for the created incident. `{signal_type}` is substituted.
    pub title_template: String,
    /// Severity to assign to the created incident.
    pub severity: IncidentSeverity,
    /// Minimum number of matching signals before an incident is opened
    /// (de-duplication / flap-suppression threshold).
    pub threshold_count: u32,
}

impl IncidentRule {
    /// Create a rule that fires on the first matching signal.
    pub fn immediate(
        id: impl Into<String>,
        signal_type_prefix: impl Into<String>,
        title_template: impl Into<String>,
        severity: IncidentSeverity,
    ) -> Self {
        Self {
            id: id.into(),
            signal_type_prefix: signal_type_prefix.into(),
            title_template: title_template.into(),
            severity,
            threshold_count: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Incident
// ---------------------------------------------------------------------------

/// An automatically created or manually opened incident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Incident {
    /// Unique incident ID (UUID v4).
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Severity classification.
    pub severity: IncidentSeverity,
    /// Current lifecycle state.
    pub state: IncidentState,
    /// Dimension that owns this incident.
    pub dimension_id: String,
    /// Rule ID that created this incident (if any).
    pub created_by_rule: Option<String>,
    /// Signals that contributed to opening this incident.
    pub signals: Vec<TelemetrySignal>,
    /// Unix timestamp (seconds) when opened.
    pub opened_at: u64,
    /// Unix timestamp (seconds) when resolved, if applicable.
    pub resolved_at: Option<u64>,
    /// Free-form labels (e.g. `{"owner": "platform-team"}`).
    pub labels: HashMap<String, String>,
}

/// Lifecycle states for an incident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IncidentState {
    /// Incident is open and under investigation.
    Open,
    /// Incident is acknowledged but not yet resolved.
    Acknowledged,
    /// Incident has been resolved.
    Resolved,
}

impl Incident {
    fn new(
        title: impl Into<String>,
        severity: IncidentSeverity,
        dimension_id: impl Into<String>,
        created_by_rule: Option<String>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            severity,
            state: IncidentState::Open,
            dimension_id: dimension_id.into(),
            created_by_rule,
            signals: Vec::new(),
            opened_at: now,
            resolved_at: None,
            labels: HashMap::new(),
        }
    }

    /// Acknowledge the incident.
    pub fn acknowledge(&mut self) {
        self.state = IncidentState::Acknowledged;
    }

    /// Resolve the incident.
    pub fn resolve(&mut self) {
        self.state = IncidentState::Resolved;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.resolved_at = Some(now);
    }

    /// Add a label to the incident.
    pub fn label(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.labels.insert(key.into(), value.into());
    }
}

// ---------------------------------------------------------------------------
// IncidentPipeline
// ---------------------------------------------------------------------------

/// Evaluates telemetry signals against incident rules and manages incidents.
#[derive(Debug, Default)]
pub struct IncidentPipeline {
    rules: Vec<IncidentRule>,
    incidents: HashMap<String, Incident>,
    /// Signal counts per (rule_id, dimension_id) pair for threshold tracking.
    signal_counts: HashMap<(String, String), u32>,
}

impl IncidentPipeline {
    /// Create a new empty pipeline.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new incident rule.
    pub fn add_rule(&mut self, rule: IncidentRule) -> Result<(), IncidentError> {
        if self.rules.iter().any(|r| r.id == rule.id) {
            return Err(IncidentError::DuplicateRule(rule.id.clone()));
        }
        self.rules.push(rule);
        Ok(())
    }

    /// Feed a telemetry signal into the pipeline.
    ///
    /// For each matching rule, the signal count is incremented. Once the
    /// threshold is reached a new incident is opened (or the existing open
    /// incident for that rule+dimension has the signal appended).
    ///
    /// Returns the IDs of any incidents that were opened or updated.
    pub fn ingest(&mut self, signal: TelemetrySignal) -> Vec<String> {
        let mut affected = Vec::new();

        let matching_rules: Vec<IncidentRule> = self
            .rules
            .iter()
            .filter(|r| signal.signal_type.starts_with(&r.signal_type_prefix))
            .cloned()
            .collect();

        for rule in matching_rules {
            let count_key = (rule.id.clone(), signal.dimension_id.clone());
            let count = self.signal_counts.entry(count_key).or_insert(0);
            *count += 1;

            if *count < rule.threshold_count {
                continue;
            }

            // Check if we already have an open incident for this rule + dimension.
            let existing = self.incidents.values_mut().find(|inc| {
                inc.created_by_rule.as_deref() == Some(&rule.id)
                    && inc.dimension_id == signal.dimension_id
                    && inc.state == IncidentState::Open
            });

            if let Some(inc) = existing {
                inc.signals.push(signal.clone());
                affected.push(inc.id.clone());
                info!(incident_id = %inc.id, signal_type = %signal.signal_type, "incident.signal_appended");
            } else {
                let title = rule
                    .title_template
                    .replace("{signal_type}", &signal.signal_type);
                let mut inc = Incident::new(
                    &title,
                    rule.severity,
                    signal.dimension_id.clone(),
                    Some(rule.id.clone()),
                );
                inc.signals.push(signal.clone());
                let id = inc.id.clone();
                warn!(
                    incident_id = %id,
                    title = %title,
                    severity = ?rule.severity,
                    "incident.opened"
                );
                self.incidents.insert(id.clone(), inc);
                affected.push(id);
            }
        }

        affected
    }

    /// Retrieve an incident by ID.
    pub fn get(&self, id: &str) -> Option<&Incident> {
        self.incidents.get(id)
    }

    /// Retrieve a mutable reference to an incident by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Incident> {
        self.incidents.get_mut(id)
    }

    /// Return all open incidents.
    pub fn open_incidents(&self) -> Vec<&Incident> {
        self.incidents
            .values()
            .filter(|i| i.state == IncidentState::Open)
            .collect()
    }

    /// Return all incidents (any state).
    pub fn all_incidents(&self) -> Vec<&Incident> {
        self.incidents.values().collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_signal(signal_type: &str, dim: &str) -> TelemetrySignal {
        TelemetrySignal {
            signal_type: signal_type.to_string(),
            dimension_id: dim.to_string(),
            task_id: None,
            payload: serde_json::json!({}),
            timestamp_secs: 0,
        }
    }

    #[test]
    fn incident_created_on_matching_signal() {
        let mut pipeline = IncidentPipeline::new();
        pipeline
            .add_rule(IncidentRule::immediate(
                "r1",
                "slo.",
                "SLO breach: {signal_type}",
                IncidentSeverity::High,
            ))
            .unwrap();

        let affected = pipeline.ingest(make_signal("slo.breach", "dim-1"));
        assert_eq!(affected.len(), 1);
        let inc = pipeline.get(&affected[0]).unwrap();
        assert_eq!(inc.state, IncidentState::Open);
        assert_eq!(inc.signals.len(), 1);
    }

    #[test]
    fn signal_appended_to_open_incident() {
        let mut pipeline = IncidentPipeline::new();
        pipeline
            .add_rule(IncidentRule::immediate(
                "r1",
                "slo.",
                "SLO breach: {signal_type}",
                IncidentSeverity::High,
            ))
            .unwrap();

        let a1 = pipeline.ingest(make_signal("slo.breach", "dim-1"));
        let a2 = pipeline.ingest(make_signal("slo.breach", "dim-1"));
        // Same incident updated.
        assert_eq!(a1[0], a2[0]);
        let inc = pipeline.get(&a1[0]).unwrap();
        assert_eq!(inc.signals.len(), 2);
    }

    #[test]
    fn threshold_suppresses_early_signals() {
        let mut pipeline = IncidentPipeline::new();
        let mut rule = IncidentRule::immediate(
            "r1",
            "chaos.",
            "Chaos: {signal_type}",
            IncidentSeverity::Medium,
        );
        rule.threshold_count = 3;
        pipeline.add_rule(rule).unwrap();

        let a1 = pipeline.ingest(make_signal("chaos.fault", "dim-1"));
        let a2 = pipeline.ingest(make_signal("chaos.fault", "dim-1"));
        assert!(a1.is_empty());
        assert!(a2.is_empty());

        let a3 = pipeline.ingest(make_signal("chaos.fault", "dim-1"));
        assert_eq!(a3.len(), 1);
    }

    #[test]
    fn resolve_incident() {
        let mut pipeline = IncidentPipeline::new();
        pipeline
            .add_rule(IncidentRule::immediate(
                "r1",
                "slo.",
                "SLO breach",
                IncidentSeverity::Critical,
            ))
            .unwrap();
        let affected = pipeline.ingest(make_signal("slo.breach", "dim-1"));
        let inc = pipeline.get_mut(&affected[0]).unwrap();
        inc.resolve();
        assert_eq!(inc.state, IncidentState::Resolved);
        assert!(inc.resolved_at.is_some());
    }

    #[test]
    fn no_matching_rule_returns_empty() {
        let mut pipeline = IncidentPipeline::new();
        let affected = pipeline.ingest(make_signal("mesh.write", "dim-1"));
        assert!(affected.is_empty());
    }

    #[test]
    fn duplicate_rule_fails() {
        let mut pipeline = IncidentPipeline::new();
        let r1 = IncidentRule::immediate("r1", "slo.", "x", IncidentSeverity::Low);
        let r2 = IncidentRule::immediate("r1", "slo.", "y", IncidentSeverity::Low);
        pipeline.add_rule(r1).unwrap();
        assert!(pipeline.add_rule(r2).is_err());
    }
}
