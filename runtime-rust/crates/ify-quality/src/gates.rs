//! # gates — Quality Gates for Merge Readiness
//!
//! Defines the set of quality gates that every merge request must satisfy
//! before it may be merged.  Each gate has a name, a category, and a
//! predicate evaluated against a [`MergeReadinessReport`].
//!
//! The canonical gate set is available via [`QualityGateSet::canonical`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the quality gates module.
#[derive(Debug, Error)]
pub enum GateError {
    /// A gate with the given name is already registered.
    #[error("duplicate gate name: {0}")]
    DuplicateName(String),
    /// A required metric is missing from the report.
    #[error("missing metric '{0}' in merge-readiness report")]
    MissingMetric(String),
}

// ---------------------------------------------------------------------------
// Gate categories
// ---------------------------------------------------------------------------

/// High-level category for a quality gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateCategory {
    /// All required test suites must pass.
    TestPassing,
    /// Code coverage must meet the minimum threshold.
    Coverage,
    /// No new security findings above the configured severity.
    Security,
    /// No performance regressions above the allowed budget.
    Performance,
    /// Contract / IDL conformance tests must pass.
    ContractConformance,
    /// Change-log and documentation must be present.
    Documentation,
}

// ---------------------------------------------------------------------------
// Merge-readiness report
// ---------------------------------------------------------------------------

/// A collection of named metric values produced by the CI pipeline.
///
/// CI must populate these values before the gate set is evaluated.  Keys are
/// metric names; values are floating-point measurements (e.g., percentages,
/// counts, booleans encoded as 0.0/1.0).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MergeReadinessReport {
    metrics: HashMap<String, f64>,
}

impl MergeReadinessReport {
    /// Create an empty report.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a metric value.
    pub fn record(&mut self, name: impl Into<String>, value: f64) {
        self.metrics.insert(name.into(), value);
    }

    /// Retrieve a metric value.
    pub fn get(&self, name: &str) -> Option<f64> {
        self.metrics.get(name).copied()
    }
}

// ---------------------------------------------------------------------------
// Gate definition
// ---------------------------------------------------------------------------

/// Comparison operator used by a gate threshold check.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateOp {
    /// The metric must be ≥ threshold.
    AtLeast,
    /// The metric must be ≤ threshold.
    AtMost,
    /// The metric must be == threshold (uses approximate equality ε = 1e-9).
    Equal,
}

/// A single quality gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityGate {
    /// Unique gate name.
    pub name: String,
    /// Category.
    pub category: GateCategory,
    /// The CI metric name this gate reads from the report.
    pub metric: String,
    /// Comparison operator.
    pub op: GateOp,
    /// Threshold value.
    pub threshold: f64,
    /// Human-readable description of why this gate exists.
    pub rationale: String,
}

impl QualityGate {
    /// Create a new gate.
    pub fn new(
        name: impl Into<String>,
        category: GateCategory,
        metric: impl Into<String>,
        op: GateOp,
        threshold: f64,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            category,
            metric: metric.into(),
            op,
            threshold,
            rationale: rationale.into(),
        }
    }

    /// Evaluate this gate against a report.
    ///
    /// # Errors
    /// Returns [`GateError::MissingMetric`] if the metric is not present.
    pub fn evaluate(&self, report: &MergeReadinessReport) -> Result<GateOutcome, GateError> {
        let value = report
            .get(&self.metric)
            .ok_or_else(|| GateError::MissingMetric(self.metric.clone()))?;

        let passed = match self.op {
            GateOp::AtLeast => value >= self.threshold,
            GateOp::AtMost => value <= self.threshold,
            GateOp::Equal => (value - self.threshold).abs() < 1e-9,
        };

        Ok(GateOutcome {
            gate_name: self.name.clone(),
            passed,
            metric_value: value,
            threshold: self.threshold,
        })
    }
}

// ---------------------------------------------------------------------------
// Gate outcome
// ---------------------------------------------------------------------------

/// The result of evaluating a single quality gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateOutcome {
    /// Gate name.
    pub gate_name: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// The actual metric value at evaluation time.
    pub metric_value: f64,
    /// The required threshold.
    pub threshold: f64,
}

// ---------------------------------------------------------------------------
// Gate set
// ---------------------------------------------------------------------------

/// A registry of quality gates evaluated as a group.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityGateSet {
    gates: Vec<QualityGate>,
}

impl QualityGateSet {
    /// Build the canonical infinityOS quality gate set.
    pub fn canonical() -> Self {
        let mut gs = Self::default();

        let _ = gs.add(QualityGate::new(
            "all-unit-tests-pass",
            GateCategory::TestPassing,
            "unit_tests_passed",
            GateOp::Equal,
            1.0,
            "All unit tests must pass (1.0 = pass, 0.0 = failure)",
        ));

        let _ = gs.add(QualityGate::new(
            "all-integration-tests-pass",
            GateCategory::TestPassing,
            "integration_tests_passed",
            GateOp::Equal,
            1.0,
            "All integration tests must pass",
        ));

        let _ = gs.add(QualityGate::new(
            "line-coverage-min-80",
            GateCategory::Coverage,
            "line_coverage_pct",
            GateOp::AtLeast,
            80.0,
            "Line coverage must be at least 80 %",
        ));

        let _ = gs.add(QualityGate::new(
            "branch-coverage-min-70",
            GateCategory::Coverage,
            "branch_coverage_pct",
            GateOp::AtLeast,
            70.0,
            "Branch coverage must be at least 70 %",
        ));

        let _ = gs.add(QualityGate::new(
            "no-critical-security-findings",
            GateCategory::Security,
            "critical_security_findings",
            GateOp::AtMost,
            0.0,
            "Zero critical security findings allowed",
        ));

        let _ = gs.add(QualityGate::new(
            "no-high-security-findings",
            GateCategory::Security,
            "high_security_findings",
            GateOp::AtMost,
            0.0,
            "Zero high-severity security findings allowed",
        ));

        let _ = gs.add(QualityGate::new(
            "p99-latency-regression-max-20pct",
            GateCategory::Performance,
            "p99_latency_regression_pct",
            GateOp::AtMost,
            20.0,
            "p99 latency must not regress by more than 20 % vs baseline",
        ));

        let _ = gs.add(QualityGate::new(
            "throughput-regression-max-10pct",
            GateCategory::Performance,
            "throughput_regression_pct",
            GateOp::AtMost,
            10.0,
            "Throughput must not regress by more than 10 % vs baseline",
        ));

        let _ = gs.add(QualityGate::new(
            "contract-conformance-pass",
            GateCategory::ContractConformance,
            "contract_conformance_passed",
            GateOp::Equal,
            1.0,
            "All IDL contract conformance tests must pass",
        ));

        let _ = gs.add(QualityGate::new(
            "changelog-present",
            GateCategory::Documentation,
            "changelog_entry_present",
            GateOp::Equal,
            1.0,
            "A CHANGELOG entry describing the change must be present",
        ));

        gs
    }

    /// Add a gate to the set.
    ///
    /// # Errors
    /// Returns [`GateError::DuplicateName`] if a gate with the same name already exists.
    pub fn add(&mut self, gate: QualityGate) -> Result<(), GateError> {
        if self.gates.iter().any(|g| g.name == gate.name) {
            return Err(GateError::DuplicateName(gate.name));
        }
        self.gates.push(gate);
        Ok(())
    }

    /// Return all gates.
    pub fn gates(&self) -> &[QualityGate] {
        &self.gates
    }

    /// Evaluate all gates against a report and return the combined verdict.
    ///
    /// Individual evaluation errors (such as `MissingMetric`) are treated as
    /// gate failures (the metric was not produced or the gate could not be
    /// evaluated, so the gate cannot pass).
    pub fn evaluate_all(&self, report: &MergeReadinessReport) -> GateSetVerdict {
        let mut outcomes = Vec::with_capacity(self.gates.len());
        for gate in &self.gates {
            let outcome = match gate.evaluate(report) {
                Ok(o) => o,
                Err(_) => GateOutcome {
                    gate_name: gate.name.clone(),
                    passed: false,
                    metric_value: 0.0,
                    threshold: gate.threshold,
                },
            };
            outcomes.push(outcome);
        }
        let all_passed = outcomes.iter().all(|o| o.passed);
        GateSetVerdict { all_passed, outcomes }
    }
}

/// The aggregated result of running the full gate set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateSetVerdict {
    /// `true` iff every gate passed.
    pub all_passed: bool,
    /// Individual outcomes, one per gate.
    pub outcomes: Vec<GateOutcome>,
}

impl GateSetVerdict {
    /// Return the gates that failed.
    pub fn failed_gates(&self) -> impl Iterator<Item = &GateOutcome> {
        self.outcomes.iter().filter(|o| !o.passed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_report() -> MergeReadinessReport {
        let mut r = MergeReadinessReport::new();
        r.record("unit_tests_passed", 1.0);
        r.record("integration_tests_passed", 1.0);
        r.record("line_coverage_pct", 85.0);
        r.record("branch_coverage_pct", 72.0);
        r.record("critical_security_findings", 0.0);
        r.record("high_security_findings", 0.0);
        r.record("p99_latency_regression_pct", 5.0);
        r.record("throughput_regression_pct", 3.0);
        r.record("contract_conformance_passed", 1.0);
        r.record("changelog_entry_present", 1.0);
        r
    }

    #[test]
    fn canonical_gates_all_pass_on_healthy_report() {
        let gs = QualityGateSet::canonical();
        let verdict = gs.evaluate_all(&passing_report());
        assert!(verdict.all_passed, "failed gates: {:?}", verdict.failed_gates().collect::<Vec<_>>());
    }

    #[test]
    fn low_coverage_fails_gate() {
        let gs = QualityGateSet::canonical();
        let mut r = passing_report();
        r.record("line_coverage_pct", 75.0); // below 80 %
        let verdict = gs.evaluate_all(&r);
        assert!(!verdict.all_passed);
        assert!(verdict.failed_gates().any(|o| o.gate_name == "line-coverage-min-80"));
    }

    #[test]
    fn security_finding_fails_gate() {
        let gs = QualityGateSet::canonical();
        let mut r = passing_report();
        r.record("critical_security_findings", 1.0);
        let verdict = gs.evaluate_all(&r);
        assert!(!verdict.all_passed);
    }

    #[test]
    fn missing_metric_causes_gate_failure() {
        let gs = QualityGateSet::canonical();
        let report = MergeReadinessReport::new(); // empty — all metrics missing
        let verdict = gs.evaluate_all(&report);
        assert!(!verdict.all_passed);
    }

    #[test]
    fn duplicate_gate_name_is_rejected() {
        let mut gs = QualityGateSet::default();
        let gate = QualityGate::new("my-gate", GateCategory::TestPassing, "m", GateOp::Equal, 1.0, "r");
        gs.add(gate.clone()).unwrap();
        assert!(gs.add(gate).is_err());
    }

    #[test]
    fn verdict_serialises_roundtrip() {
        let gs = QualityGateSet::canonical();
        let verdict = gs.evaluate_all(&passing_report());
        let json = serde_json::to_string(&verdict).unwrap();
        let back: GateSetVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(back.all_passed, verdict.all_passed);
    }
}
