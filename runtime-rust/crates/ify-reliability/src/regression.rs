//! Regression triage workflow — Epic K item 8.
//!
//! Provides the data structures and engine for triaging regressions:
//! - [`RegressionReport`] — a detected regression with metadata.
//! - [`TriageLabel`] — classification labels (severity, component, status).
//! - [`TriageOwner`] — the team or individual accountable for resolution.
//! - [`SlaTier`] — the SLA time-to-resolution tier.
//! - [`RegressionTriageEngine`] — assigns labels, owners, and SLA deadlines
//!   to incoming regression reports, and tracks resolution status.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the regression triage subsystem.
#[derive(Debug, Error)]
pub enum TriageError {
    /// A regression with the given ID already exists.
    #[error("regression '{0}' already exists")]
    Duplicate(String),
    /// Regression not found.
    #[error("regression '{0}' not found")]
    NotFound(String),
    /// An owner assignment rule with the given ID already exists.
    #[error("owner rule '{0}' already registered")]
    DuplicateRule(String),
}

// ---------------------------------------------------------------------------
// SlaTier
// ---------------------------------------------------------------------------

/// SLA tier determining the maximum time-to-resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlaTier {
    /// P0 — resolve within 4 hours.
    P0,
    /// P1 — resolve within 24 hours.
    P1,
    /// P2 — resolve within 3 days.
    P2,
    /// P3 — resolve within 2 weeks.
    P3,
}

impl SlaTier {
    /// Resolution deadline in seconds from now.
    pub fn deadline_secs(&self) -> u64 {
        match self {
            SlaTier::P0 => 4 * 3600,
            SlaTier::P1 => 24 * 3600,
            SlaTier::P2 => 3 * 24 * 3600,
            SlaTier::P3 => 14 * 24 * 3600,
        }
    }

    /// Human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            SlaTier::P0 => "P0 – resolve within 4 h",
            SlaTier::P1 => "P1 – resolve within 24 h",
            SlaTier::P2 => "P2 – resolve within 3 days",
            SlaTier::P3 => "P3 – resolve within 2 weeks",
        }
    }
}

// ---------------------------------------------------------------------------
// TriageLabel
// ---------------------------------------------------------------------------

/// A label applied to a regression during triage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TriageLabel {
    /// Label key (e.g. `"component"`, `"kind"`, `"status"`).
    pub key: String,
    /// Label value (e.g. `"scheduler"`, `"latency"`, `"triaged"`).
    pub value: String,
}

impl TriageLabel {
    /// Create a new label.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// TriageOwner
// ---------------------------------------------------------------------------

/// Accountable owner for a regression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageOwner {
    /// Team or individual identifier.
    pub owner: String,
    /// Optional contact channel (e.g. Slack channel, email alias).
    pub contact: Option<String>,
}

impl TriageOwner {
    /// Create a new owner.
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            contact: None,
        }
    }

    /// Create an owner with a contact channel.
    pub fn with_contact(owner: impl Into<String>, contact: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            contact: Some(contact.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// RegressionReport
// ---------------------------------------------------------------------------

/// Current triage state of a regression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageState {
    /// Newly detected, not yet triaged.
    Untriaged,
    /// Assigned to an owner and SLA tier; actively tracked.
    Triaged,
    /// Fix is in progress.
    InProgress,
    /// Regression has been resolved.
    Resolved,
    /// Accepted as a known limitation; will not fix in this cycle.
    WontFix,
}

/// A detected regression with triage metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionReport {
    /// Unique regression ID.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Affected component or subsystem (e.g. `"scheduler"`, `"mesh"`, `"ui"`).
    pub component: String,
    /// Kaizen cycle in which this regression was detected (e.g. `"2026-W11"`).
    pub cycle_id: String,
    /// Severity measured value that triggered the regression.
    pub measured_value: f64,
    /// Baseline value that was exceeded.
    pub baseline_value: f64,
    /// Current triage state.
    pub state: TriageState,
    /// Labels assigned by the triage engine.
    pub labels: Vec<TriageLabel>,
    /// Owner assigned by the triage engine.
    pub owner: Option<TriageOwner>,
    /// SLA tier assigned by the triage engine.
    pub sla: Option<SlaTier>,
    /// Unix timestamp of detection.
    pub detected_at: u64,
    /// Unix timestamp of resolution (if resolved).
    pub resolved_at: Option<u64>,
}

impl RegressionReport {
    /// Create a new untriaged regression report.
    pub fn new(
        title: impl Into<String>,
        component: impl Into<String>,
        cycle_id: impl Into<String>,
        measured_value: f64,
        baseline_value: f64,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            component: component.into(),
            cycle_id: cycle_id.into(),
            measured_value,
            baseline_value,
            state: TriageState::Untriaged,
            labels: Vec::new(),
            owner: None,
            sla: None,
            detected_at: now,
            resolved_at: None,
        }
    }

    /// The regression ratio: `(measured - baseline) / baseline`.
    ///
    /// Returns `f64::INFINITY` when `baseline_value` is zero.  Callers
    /// should check for this sentinel before using the value in arithmetic
    /// that would propagate infinity (e.g. when comparing ratios, treat
    /// infinity as "maximum severity").
    pub fn regression_ratio(&self) -> f64 {
        if self.baseline_value == 0.0 {
            return f64::INFINITY;
        }
        (self.measured_value - self.baseline_value) / self.baseline_value
    }
}

// ---------------------------------------------------------------------------
// OwnerRule
// ---------------------------------------------------------------------------

/// A rule that maps a component prefix to a triage owner and SLA tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerRule {
    /// Rule ID.
    pub id: String,
    /// Component prefix to match (e.g. `"scheduler"`, `"mesh"`).
    pub component_prefix: String,
    /// Owner to assign.
    pub owner: TriageOwner,
    /// Default SLA tier to assign.
    pub default_sla: SlaTier,
}

// ---------------------------------------------------------------------------
// RegressionTriageEngine
// ---------------------------------------------------------------------------

/// Assigns owners, labels, and SLA tiers to incoming regression reports.
#[derive(Debug, Default)]
pub struct RegressionTriageEngine {
    reports: HashMap<String, RegressionReport>,
    owner_rules: Vec<OwnerRule>,
}

impl RegressionTriageEngine {
    /// Create a new empty engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an owner assignment rule.
    pub fn add_owner_rule(&mut self, rule: OwnerRule) -> Result<(), TriageError> {
        if self.owner_rules.iter().any(|r| r.id == rule.id) {
            return Err(TriageError::DuplicateRule(rule.id.clone()));
        }
        self.owner_rules.push(rule);
        Ok(())
    }

    /// Submit a regression report for triage.
    ///
    /// The engine automatically:
    /// 1. Assigns owner and SLA from matching owner rules.
    /// 2. Labels the report with `component` and `severity` based on the
    ///    regression ratio.
    /// 3. Transitions state to `Triaged`.
    pub fn submit(&mut self, mut report: RegressionReport) -> Result<String, TriageError> {
        if self.reports.contains_key(&report.id) {
            return Err(TriageError::Duplicate(report.id.clone()));
        }

        // Assign owner and SLA from first matching rule.
        if let Some(rule) = self
            .owner_rules
            .iter()
            .find(|r| report.component.starts_with(&r.component_prefix))
        {
            report.owner = Some(rule.owner.clone());
            if report.sla.is_none() {
                report.sla = Some(rule.default_sla);
            }
        }

        // Derive a severity label from the regression ratio.
        let ratio = report.regression_ratio();
        let severity = if ratio >= 0.5 {
            "critical"
        } else if ratio >= 0.2 {
            "high"
        } else if ratio >= 0.1 {
            "medium"
        } else {
            "low"
        };
        report
            .labels
            .push(TriageLabel::new("severity", severity));
        report
            .labels
            .push(TriageLabel::new("component", report.component.clone()));
        report
            .labels
            .push(TriageLabel::new("status", "triaged"));
        report.state = TriageState::Triaged;

        info!(
            id = %report.id,
            title = %report.title,
            component = %report.component,
            severity,
            "regression.triaged"
        );

        let id = report.id.clone();
        self.reports.insert(id.clone(), report);
        Ok(id)
    }

    /// Transition a regression to `InProgress`.
    pub fn start_fix(&mut self, id: &str) -> Result<(), TriageError> {
        let report = self
            .reports
            .get_mut(id)
            .ok_or_else(|| TriageError::NotFound(id.to_string()))?;
        report.state = TriageState::InProgress;
        report
            .labels
            .retain(|l| l.key != "status");
        report.labels.push(TriageLabel::new("status", "in_progress"));
        Ok(())
    }

    /// Resolve a regression.
    pub fn resolve(&mut self, id: &str) -> Result<(), TriageError> {
        let report = self
            .reports
            .get_mut(id)
            .ok_or_else(|| TriageError::NotFound(id.to_string()))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        report.state = TriageState::Resolved;
        report.resolved_at = Some(now);
        report.labels.retain(|l| l.key != "status");
        report.labels.push(TriageLabel::new("status", "resolved"));
        Ok(())
    }

    /// Retrieve a report by ID.
    pub fn get(&self, id: &str) -> Option<&RegressionReport> {
        self.reports.get(id)
    }

    /// Return all reports for a given Kaizen cycle.
    pub fn by_cycle(&self, cycle_id: &str) -> Vec<&RegressionReport> {
        self.reports
            .values()
            .filter(|r| r.cycle_id == cycle_id)
            .collect()
    }

    /// Return all open (non-resolved, non-wont-fix) reports.
    pub fn open_reports(&self) -> Vec<&RegressionReport> {
        self.reports
            .values()
            .filter(|r| {
                !matches!(r.state, TriageState::Resolved | TriageState::WontFix)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(component: &str) -> RegressionReport {
        RegressionReport::new(
            "p99 latency regression",
            component,
            "2026-W11",
            300.0, // measured ms
            200.0, // baseline ms  → 50% regression
        )
    }

    #[test]
    fn regression_ratio_calculation() {
        let r = sample_report("scheduler");
        assert!((r.regression_ratio() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn submit_assigns_severity_label() {
        let mut engine = RegressionTriageEngine::new();
        let id = engine.submit(sample_report("scheduler")).unwrap();
        let report = engine.get(&id).unwrap();
        let severity_label = report
            .labels
            .iter()
            .find(|l| l.key == "severity")
            .unwrap();
        assert_eq!(severity_label.value, "critical");
    }

    #[test]
    fn submit_assigns_owner_from_rule() {
        let mut engine = RegressionTriageEngine::new();
        engine
            .add_owner_rule(OwnerRule {
                id: "r1".into(),
                component_prefix: "sched".into(),
                owner: TriageOwner::new("platform-team"),
                default_sla: SlaTier::P1,
            })
            .unwrap();
        let id = engine.submit(sample_report("scheduler")).unwrap();
        let report = engine.get(&id).unwrap();
        assert_eq!(report.owner.as_ref().unwrap().owner, "platform-team");
        assert_eq!(report.sla.unwrap(), SlaTier::P1);
    }

    #[test]
    fn resolve_transitions_state() {
        let mut engine = RegressionTriageEngine::new();
        let id = engine.submit(sample_report("mesh")).unwrap();
        engine.resolve(&id).unwrap();
        let report = engine.get(&id).unwrap();
        assert_eq!(report.state, TriageState::Resolved);
        assert!(report.resolved_at.is_some());
    }

    #[test]
    fn start_fix_transitions_state() {
        let mut engine = RegressionTriageEngine::new();
        let id = engine.submit(sample_report("ui")).unwrap();
        engine.start_fix(&id).unwrap();
        let report = engine.get(&id).unwrap();
        assert_eq!(report.state, TriageState::InProgress);
    }

    #[test]
    fn sla_deadline_values() {
        assert_eq!(SlaTier::P0.deadline_secs(), 4 * 3600);
        assert_eq!(SlaTier::P1.deadline_secs(), 24 * 3600);
    }
}
