//! # security_testing — SAST/DAST Pipeline Definitions
//!
//! Defines the configuration, finding types, and pipeline runner for the
//! infinityOS security testing pipeline.  This module covers both:
//!
//! - **SAST** (Static Application Security Testing): source-level analysis
//!   tools such as `cargo audit`, `cargo deny`, Semgrep, and CodeQL.
//! - **DAST** (Dynamic Application Security Testing): runtime probing of
//!   HTTP/RPC surfaces with tools such as OWASP ZAP.
//!
//! The pipeline is designed to be run in CI; findings are recorded as
//! structured [`SecurityFinding`] values that feed into the quality gate
//! (`no-critical-security-findings`) and the test reporting widget.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the security testing module.
#[derive(Debug, Error)]
pub enum SecurityTestingError {
    /// A scanner with the given name is already registered.
    #[error("duplicate scanner: {0}")]
    DuplicateScanner(String),
}

// ---------------------------------------------------------------------------
// Finding severity
// ---------------------------------------------------------------------------

/// Severity level of a security finding, aligned with CVSS categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    /// Informational: no immediate risk.
    Info,
    /// Low: minimal risk, fix when convenient.
    Low,
    /// Medium: moderate risk, fix before next release.
    Medium,
    /// High: significant risk, fix before merge.
    High,
    /// Critical: exploitable, block merge immediately.
    Critical,
}

// ---------------------------------------------------------------------------
// Pipeline type
// ---------------------------------------------------------------------------

/// Whether a scanner performs static or dynamic analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PipelineKind {
    /// Static Application Security Testing.
    Sast,
    /// Dynamic Application Security Testing.
    Dast,
    /// Software Composition Analysis (dependency vulnerabilities).
    Sca,
}

// ---------------------------------------------------------------------------
// Scanner configuration
// ---------------------------------------------------------------------------

/// Configuration for a single security scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    /// Unique scanner name.
    pub name: String,
    /// Pipeline type.
    pub kind: PipelineKind,
    /// Command or tool invocation (informational; actual execution is external).
    pub command: String,
    /// Minimum severity that causes a gate failure.
    pub fail_on_severity: FindingSeverity,
    /// Whether this scanner is enabled in CI.
    pub ci_enabled: bool,
    /// Arbitrary extra options passed to the tool.
    pub options: HashMap<String, String>,
}

impl ScannerConfig {
    /// Create a new scanner configuration.
    pub fn new(
        name: impl Into<String>,
        kind: PipelineKind,
        command: impl Into<String>,
        fail_on_severity: FindingSeverity,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            command: command.into(),
            fail_on_severity,
            ci_enabled: true,
            options: HashMap::new(),
        }
    }

    /// Attach an extra option.
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Security finding
// ---------------------------------------------------------------------------

/// A single finding produced by a security scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    /// Name of the scanner that produced this finding.
    pub scanner: String,
    /// Finding severity.
    pub severity: FindingSeverity,
    /// Short title.
    pub title: String,
    /// Longer description (may include remediation advice).
    pub description: String,
    /// Source location (file path and optional line number).
    pub location: Option<String>,
    /// CVE or advisory ID if applicable.
    pub advisory_id: Option<String>,
}

impl SecurityFinding {
    /// Create a new finding.
    pub fn new(
        scanner: impl Into<String>,
        severity: FindingSeverity,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            scanner: scanner.into(),
            severity,
            title: title.into(),
            description: description.into(),
            location: None,
            advisory_id: None,
        }
    }

    /// Attach a source location.
    pub fn at(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    /// Attach an advisory identifier.
    pub fn with_advisory(mut self, id: impl Into<String>) -> Self {
        self.advisory_id = Some(id.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Pipeline configuration
// ---------------------------------------------------------------------------

/// The full SAST/DAST pipeline configuration for infinityOS.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityPipeline {
    scanners: HashMap<String, ScannerConfig>,
}

impl SecurityPipeline {
    /// Build the canonical infinityOS security testing pipeline.
    pub fn canonical() -> Self {
        let mut p = Self::default();

        // SAST — Rust advisory/vulnerability database check.
        let _ = p.add(ScannerConfig::new(
            "cargo-audit",
            PipelineKind::Sca,
            "cargo audit --deny warnings",
            FindingSeverity::High,
        ));

        // SAST — deny.toml license + ban enforcement.
        let _ = p.add(ScannerConfig::new(
            "cargo-deny",
            PipelineKind::Sast,
            "cargo deny check",
            FindingSeverity::High,
        ));

        // SAST — Semgrep rules for Rust-specific patterns.
        let _ = p.add(ScannerConfig::new(
            "semgrep-rust",
            PipelineKind::Sast,
            "semgrep --config=p/rust --error",
            FindingSeverity::High,
        )
        .with_option("config", "p/rust"));

        // SAST — CodeQL for cross-language analysis (C + Rust).
        let _ = p.add(ScannerConfig::new(
            "codeql",
            PipelineKind::Sast,
            "codeql database analyze --format=sarif-latest",
            FindingSeverity::Critical,
        ));

        // DAST — OWASP ZAP baseline scan against the local API surface.
        let _ = p.add(ScannerConfig::new(
            "owasp-zap-baseline",
            PipelineKind::Dast,
            "zap-baseline.py -t http://localhost:8080",
            FindingSeverity::High,
        )
        .with_option("risk-level", "medium"));

        p
    }

    /// Add a scanner configuration.
    ///
    /// # Errors
    /// Returns [`SecurityTestingError::DuplicateScanner`] if already registered.
    pub fn add(&mut self, config: ScannerConfig) -> Result<(), SecurityTestingError> {
        if self.scanners.contains_key(&config.name) {
            return Err(SecurityTestingError::DuplicateScanner(config.name));
        }
        self.scanners.insert(config.name.clone(), config);
        Ok(())
    }

    /// Return all scanner configurations.
    pub fn scanners(&self) -> impl Iterator<Item = &ScannerConfig> {
        self.scanners.values()
    }

    /// Return CI-enabled scanners only.
    pub fn ci_scanners(&self) -> impl Iterator<Item = &ScannerConfig> {
        self.scanners.values().filter(|s| s.ci_enabled)
    }

    /// Look up a scanner by name.
    pub fn get(&self, name: &str) -> Option<&ScannerConfig> {
        self.scanners.get(name)
    }

    /// Evaluate a set of findings against the configured scanner thresholds.
    ///
    /// Findings from **unknown scanners** (not registered in this pipeline) are
    /// treated as blocking to prevent misconfiguration (e.g., renamed scanners)
    /// from silently producing a false `passed` result.
    pub fn evaluate(&self, findings: &[SecurityFinding]) -> PipelineEvaluation {
        let mut blocking_count = 0usize;
        let mut unknown_scanner_count = 0usize;
        for finding in findings {
            match self.scanners.get(&finding.scanner) {
                Some(scanner) => {
                    if finding.severity >= scanner.fail_on_severity {
                        blocking_count += 1;
                    }
                }
                None => {
                    // Unknown scanner: treat as blocking to surface config drift.
                    unknown_scanner_count += 1;
                    blocking_count += 1;
                }
            }
        }
        PipelineEvaluation {
            passed: blocking_count == 0,
            total_findings: findings.len(),
            blocking_count,
            unknown_scanner_count,
        }
    }
}

/// The result of evaluating a set of security findings against the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineEvaluation {
    /// `true` iff no blocking findings were found.
    pub passed: bool,
    /// Total number of findings (all severities).
    pub total_findings: usize,
    /// Number of findings that block the pipeline (including unknown scanners).
    pub blocking_count: usize,
    /// Number of findings from scanners not registered in this pipeline.
    /// A non-zero value indicates a configuration drift (e.g., a renamed scanner).
    pub unknown_scanner_count: usize,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_pipeline_has_expected_scanners() {
        let p = SecurityPipeline::canonical();
        assert!(p.get("cargo-audit").is_some());
        assert!(p.get("cargo-deny").is_some());
        assert!(p.get("semgrep-rust").is_some());
        assert!(p.get("codeql").is_some());
        assert!(p.get("owasp-zap-baseline").is_some());
    }

    #[test]
    fn empty_findings_passes() {
        let p = SecurityPipeline::canonical();
        let eval = p.evaluate(&[]);
        assert!(eval.passed);
        assert_eq!(eval.blocking_count, 0);
    }

    #[test]
    fn critical_finding_blocks_pipeline() {
        let p = SecurityPipeline::canonical();
        let finding = SecurityFinding::new(
            "cargo-audit",
            FindingSeverity::Critical,
            "CVE-0000-0000",
            "Known vulnerability in dependency X",
        );
        let eval = p.evaluate(&[finding]);
        assert!(!eval.passed);
        assert_eq!(eval.blocking_count, 1);
    }

    #[test]
    fn low_severity_finding_does_not_block_cargo_audit() {
        let p = SecurityPipeline::canonical();
        // cargo-audit fails on High+; Low should not block.
        let finding = SecurityFinding::new(
            "cargo-audit",
            FindingSeverity::Low,
            "advisory-low",
            "Low severity advisory",
        );
        let eval = p.evaluate(&[finding]);
        assert!(eval.passed);
    }

    #[test]
    fn duplicate_scanner_is_rejected() {
        let mut p = SecurityPipeline::default();
        let sc = ScannerConfig::new("s", PipelineKind::Sast, "cmd", FindingSeverity::High);
        p.add(sc.clone()).unwrap();
        assert!(p.add(sc).is_err());
    }

    #[test]
    fn severity_ordering_is_correct() {
        assert!(FindingSeverity::Critical > FindingSeverity::High);
        assert!(FindingSeverity::High > FindingSeverity::Medium);
        assert!(FindingSeverity::Medium > FindingSeverity::Low);
        assert!(FindingSeverity::Low > FindingSeverity::Info);
    }

    #[test]
    fn unknown_scanner_blocks_pipeline() {
        let p = SecurityPipeline::canonical();
        let finding = SecurityFinding::new(
            "unregistered-scanner",
            FindingSeverity::Low,
            "some-finding",
            "Finding from an unregistered scanner",
        );
        let eval = p.evaluate(&[finding]);
        assert!(!eval.passed);
        assert_eq!(eval.unknown_scanner_count, 1);
        assert_eq!(eval.blocking_count, 1);
    }
}
