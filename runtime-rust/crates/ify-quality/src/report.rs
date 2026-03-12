//! # report — Test Reporting Widget
//!
//! Aggregates results from all test types (unit, integration, performance,
//! fuzz, security, contract, golden, load) into a single
//! [`TestReportSnapshot`] that can be rendered in the infinityOS reliability
//! dashboard widget or exported as a JSON artifact to the mesh.
//!
//! The reporting widget is designed to be queried after every CI run; it
//! emits a structured summary that feeds into quality gate evaluation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the test reporting module.
#[derive(Debug, Error)]
pub enum ReportError {
    /// A suite entry with the given name already exists.
    #[error("duplicate suite report: {0}")]
    DuplicateSuite(String),
}

// ---------------------------------------------------------------------------
// Suite status
// ---------------------------------------------------------------------------

/// High-level pass/fail status for a test suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuiteStatus {
    /// All tests passed.
    Passed,
    /// One or more tests failed.
    Failed,
    /// The suite was skipped (e.g., not run in this CI pass).
    Skipped,
    /// The suite is still running.
    InProgress,
}

// ---------------------------------------------------------------------------
// Suite report entry
// ---------------------------------------------------------------------------

/// Per-suite result summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteReport {
    /// Suite name (matches [`TestSuiteDescriptor::name`]).
    pub name: String,
    /// Overall status.
    pub status: SuiteStatus,
    /// Number of tests/checks that passed.
    pub passed: u64,
    /// Number of tests/checks that failed.
    pub failed: u64,
    /// Number of tests/checks skipped.
    pub skipped: u64,
    /// Total elapsed time for the suite (seconds).
    pub elapsed_secs: f64,
    /// Optional line coverage percentage (if measured).
    pub line_coverage_pct: Option<f64>,
    /// Optional branch coverage percentage (if measured).
    pub branch_coverage_pct: Option<f64>,
    /// Arbitrary extra metadata (e.g., tool version, CI job URL).
    pub metadata: HashMap<String, String>,
}

impl SuiteReport {
    /// Create a passing suite report.
    pub fn passing(name: impl Into<String>, passed: u64, elapsed_secs: f64) -> Self {
        Self {
            name: name.into(),
            status: SuiteStatus::Passed,
            passed,
            failed: 0,
            skipped: 0,
            elapsed_secs,
            line_coverage_pct: None,
            branch_coverage_pct: None,
            metadata: HashMap::new(),
        }
    }

    /// Attach coverage measurements.
    pub fn with_coverage(mut self, line_pct: f64, branch_pct: f64) -> Self {
        self.line_coverage_pct = Some(line_pct);
        self.branch_coverage_pct = Some(branch_pct);
        self
    }

    /// Attach extra metadata.
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Security scan summary
// ---------------------------------------------------------------------------

/// Aggregated security finding counts by severity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityScanSummary {
    /// Number of critical findings.
    pub critical: u64,
    /// Number of high findings.
    pub high: u64,
    /// Number of medium findings.
    pub medium: u64,
    /// Number of low findings.
    pub low: u64,
    /// Number of informational findings.
    pub info: u64,
}

impl SecurityScanSummary {
    /// `true` iff there are no critical or high findings.
    pub fn is_clean(&self) -> bool {
        self.critical == 0 && self.high == 0
    }
}

// ---------------------------------------------------------------------------
// Fuzz campaign summary
// ---------------------------------------------------------------------------

/// Summary of a fuzz campaign run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzCampaignSummary {
    /// Name of the fuzz target.
    pub target: String,
    /// Number of corpus entries at the end of the campaign.
    pub corpus_size: u64,
    /// Number of unique crashes found (0 is required for gate pass).
    pub crashes: u64,
    /// Total executions performed.
    pub total_execs: u64,
    /// Executions per second.
    pub execs_per_sec: f64,
}

// ---------------------------------------------------------------------------
// Test report snapshot
// ---------------------------------------------------------------------------

/// Complete snapshot of all test results for a single CI run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestReportSnapshot {
    /// Build / run identifier (e.g., commit SHA or CI job ID).
    pub run_id: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Per-suite results.
    suite_reports: HashMap<String, SuiteReport>,
    /// Security scan summary.
    pub security: SecurityScanSummary,
    /// Fuzz campaign summaries.
    pub fuzz_campaigns: Vec<FuzzCampaignSummary>,
    /// Key scalar metrics derived from suite_reports (used by quality gates).
    pub metrics: HashMap<String, f64>,
}

impl TestReportSnapshot {
    /// Create an empty snapshot for `run_id`.
    pub fn new(run_id: impl Into<String>, timestamp: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            timestamp: timestamp.into(),
            ..Default::default()
        }
    }

    /// Add a suite report.
    ///
    /// # Errors
    /// Returns [`ReportError::DuplicateSuite`] if a report for the suite already exists.
    pub fn add_suite(&mut self, report: SuiteReport) -> Result<(), ReportError> {
        if self.suite_reports.contains_key(&report.name) {
            return Err(ReportError::DuplicateSuite(report.name));
        }
        self.suite_reports.insert(report.name.clone(), report);
        Ok(())
    }

    /// Return all suite reports.
    pub fn suite_reports(&self) -> impl Iterator<Item = &SuiteReport> {
        self.suite_reports.values()
    }

    /// Return suite reports that failed.
    pub fn failed_suites(&self) -> impl Iterator<Item = &SuiteReport> {
        self.suite_reports.values().filter(|r| r.status == SuiteStatus::Failed)
    }

    /// Derive the quality-gate metrics from the accumulated suite reports and
    /// security summary.  Must be called after all reports are added.
    pub fn derive_metrics(&mut self) {
        // unit_tests_passed / integration_tests_passed
        let unit_ok = self
            .suite_reports
            .values()
            .filter(|r| r.name.ends_with("-unit") || r.name.contains("-unit-"))
            .all(|r| r.status == SuiteStatus::Passed);
        let integration_ok = self
            .suite_reports
            .values()
            .filter(|r| r.name.contains("-integration"))
            .all(|r| r.status == SuiteStatus::Passed);

        self.metrics.insert("unit_tests_passed".into(), if unit_ok { 1.0 } else { 0.0 });
        self.metrics.insert("integration_tests_passed".into(), if integration_ok { 1.0 } else { 0.0 });

        // Aggregate coverage (average across suites that report it).
        let cov_reports: Vec<_> = self
            .suite_reports
            .values()
            .filter_map(|r| r.line_coverage_pct.map(|l| (l, r.branch_coverage_pct.unwrap_or(0.0))))
            .collect();
        if !cov_reports.is_empty() {
            let avg_line = cov_reports.iter().map(|(l, _)| l).sum::<f64>() / cov_reports.len() as f64;
            let avg_branch = cov_reports.iter().map(|(_, b)| b).sum::<f64>() / cov_reports.len() as f64;
            self.metrics.insert("line_coverage_pct".into(), avg_line);
            self.metrics.insert("branch_coverage_pct".into(), avg_branch);
        }

        // Security findings.
        self.metrics.insert("critical_security_findings".into(), self.security.critical as f64);
        self.metrics.insert("high_security_findings".into(), self.security.high as f64);

        // Fuzz crashes.
        let total_crashes: u64 = self.fuzz_campaigns.iter().map(|f| f.crashes).sum();
        self.metrics.insert("fuzz_crashes".into(), total_crashes as f64);
    }

    /// Retrieve a derived metric value.
    pub fn metric(&self, name: &str) -> Option<f64> {
        self.metrics.get(name).copied()
    }
}

// ---------------------------------------------------------------------------
// Widget renderer
// ---------------------------------------------------------------------------

/// Renders a [`TestReportSnapshot`] as a plain-text dashboard panel.
///
/// In the full infinityOS UI this renders as a dockable widget in the
/// reliability dashboard.  Here we produce a human-readable text summary
/// suitable for CI log output and snapshot testing.
pub struct TestReportWidget;

impl TestReportWidget {
    /// Render the snapshot as a multi-line text summary.
    pub fn render(snapshot: &TestReportSnapshot) -> String {
        let mut lines = vec![
            format!("=== Test Report [{}] ===", snapshot.run_id),
            format!("Timestamp: {}", snapshot.timestamp),
            String::new(),
        ];

        // Suite summary.
        lines.push("--- Suites ---".into());
        let mut suite_names: Vec<_> = snapshot.suite_reports.keys().collect();
        suite_names.sort();
        for name in suite_names {
            let r = &snapshot.suite_reports[name];
            let status_str = match r.status {
                SuiteStatus::Passed => "PASS",
                SuiteStatus::Failed => "FAIL",
                SuiteStatus::Skipped => "SKIP",
                SuiteStatus::InProgress => "RUNNING",
            };
            lines.push(format!(
                "  [{status_str}] {name} — {}/{} passed ({:.2}s)",
                r.passed,
                r.passed + r.failed,
                r.elapsed_secs,
            ));
        }

        // Security summary.
        lines.push(String::new());
        lines.push("--- Security ---".into());
        lines.push(format!(
            "  Critical: {}  High: {}  Medium: {}  Low: {}  Info: {}",
            snapshot.security.critical,
            snapshot.security.high,
            snapshot.security.medium,
            snapshot.security.low,
            snapshot.security.info,
        ));

        // Fuzz summary.
        if !snapshot.fuzz_campaigns.is_empty() {
            lines.push(String::new());
            lines.push("--- Fuzz Campaigns ---".into());
            for fc in &snapshot.fuzz_campaigns {
                lines.push(format!(
                    "  {}: corpus={} crashes={} execs={:.0}/s",
                    fc.target, fc.corpus_size, fc.crashes, fc.execs_per_sec
                ));
            }
        }

        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_derive_metrics_unit_and_integration() {
        let mut snap = TestReportSnapshot::new("sha-abc", "2026-03-12T00:00:00Z");
        snap.add_suite(SuiteReport::passing("kernel-unit", 10, 0.5).with_coverage(85.0, 72.0)).unwrap();
        snap.add_suite(SuiteReport::passing("kernel-runtime-ffi-integration", 5, 2.0)).unwrap();
        snap.derive_metrics();

        assert_eq!(snap.metric("unit_tests_passed"), Some(1.0));
        assert_eq!(snap.metric("integration_tests_passed"), Some(1.0));
        assert_eq!(snap.metric("critical_security_findings"), Some(0.0));
    }

    #[test]
    fn failed_suite_sets_unit_tests_passed_to_zero() {
        let mut snap = TestReportSnapshot::new("sha-def", "2026-03-12T00:00:00Z");
        let mut report = SuiteReport::passing("controller-unit", 10, 0.5);
        report.status = SuiteStatus::Failed;
        report.failed = 2;
        snap.add_suite(report).unwrap();
        snap.derive_metrics();
        assert_eq!(snap.metric("unit_tests_passed"), Some(0.0));
    }

    #[test]
    fn widget_render_returns_non_empty_string() {
        let mut snap = TestReportSnapshot::new("sha-123", "2026-03-12T12:00:00Z");
        snap.add_suite(SuiteReport::passing("kernel-unit", 7, 0.3)).unwrap();
        let rendered = TestReportWidget::render(&snap);
        assert!(rendered.contains("sha-123"));
        assert!(rendered.contains("kernel-unit"));
    }

    #[test]
    fn duplicate_suite_is_rejected() {
        let mut snap = TestReportSnapshot::new("x", "t");
        snap.add_suite(SuiteReport::passing("s", 1, 0.1)).unwrap();
        assert!(snap.add_suite(SuiteReport::passing("s", 1, 0.1)).is_err());
    }

    #[test]
    fn security_summary_clean_check() {
        let mut sec = SecurityScanSummary::default();
        assert!(sec.is_clean());
        sec.critical = 1;
        assert!(!sec.is_clean());
    }
}
