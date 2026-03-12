//! # strategy — Test Strategy Configuration
//!
//! Defines the infinityOS test strategy: pyramid levels, per-layer coverage
//! requirements, timing budgets, and runner configuration.  All quality agents
//! and CI pipelines reference these constants to ensure a consistent strategy
//! is applied across every layer.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors surfaced by the test strategy module.
#[derive(Debug, Error)]
pub enum StrategyError {
    /// A required configuration field is missing or invalid.
    #[error("invalid strategy configuration: {0}")]
    InvalidConfig(String),
}

// ---------------------------------------------------------------------------
// Test pyramid levels
// ---------------------------------------------------------------------------

/// The three standard levels in the infinityOS test pyramid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PyramidLevel {
    /// Fast, hermetic, in-process unit tests (< 100 ms each).
    Unit,
    /// Cross-layer integration tests using real I/O in a temp directory (< 10 s each).
    Integration,
    /// Throughput and latency benchmarks run separately with `cargo bench` or CMake bench target.
    Performance,
}

// ---------------------------------------------------------------------------
// Layer under test
// ---------------------------------------------------------------------------

/// Identifies which architectural layer a test suite belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestedLayer {
    /// C kernel: memory, scheduler, replication, FFI, service registry, trace.
    Kernel,
    /// Rust performer runtime crates (ify-core, ify-executor, ify-ffi, ify-interfaces).
    RuntimeCore,
    /// ify-controller crate (blockControllerGenerator regime).
    Controller,
    /// ify-canvas crate (UI contracts).
    Canvas,
    /// ify-reliability crate (Kaizen loop).
    Reliability,
    /// ify-security crate (threat model, audit, identity, sandbox, …).
    Security,
    /// Cross-layer integration (C ↔ Rust ABI, ActionLog, mesh flows).
    CrossLayer,
}

// ---------------------------------------------------------------------------
// Coverage requirements
// ---------------------------------------------------------------------------

/// Minimum line-coverage and branch-coverage thresholds for a layer.
///
/// Values are expressed as percentages in the range 0–100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoverageThreshold {
    /// Minimum percentage of lines that must be covered.
    pub line_coverage_pct: f64,
    /// Minimum percentage of branches that must be covered.
    pub branch_coverage_pct: f64,
}

impl CoverageThreshold {
    /// Create a new coverage threshold.
    ///
    /// # Errors
    /// Returns [`StrategyError::InvalidConfig`] if either value is outside 0–100.
    pub fn new(line_coverage_pct: f64, branch_coverage_pct: f64) -> Result<Self, StrategyError> {
        if !(0.0..=100.0).contains(&line_coverage_pct) {
            return Err(StrategyError::InvalidConfig(format!(
                "line_coverage_pct must be 0–100, got {line_coverage_pct}"
            )));
        }
        if !(0.0..=100.0).contains(&branch_coverage_pct) {
            return Err(StrategyError::InvalidConfig(format!(
                "branch_coverage_pct must be 0–100, got {branch_coverage_pct}"
            )));
        }
        Ok(Self { line_coverage_pct, branch_coverage_pct })
    }
}

// ---------------------------------------------------------------------------
// Timing budgets
// ---------------------------------------------------------------------------

/// Per-pyramid-level timing budget.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimingBudget {
    /// Maximum allowed duration per individual test (milliseconds).
    pub max_per_test_ms: u64,
    /// Maximum allowed duration for the entire suite (seconds).
    pub max_suite_secs: u64,
}

impl TimingBudget {
    /// Unit-test budget: 100 ms per test, 60 s total.
    pub const UNIT: Self = Self { max_per_test_ms: 100, max_suite_secs: 60 };
    /// Integration-test budget: 10 000 ms per test, 300 s total.
    pub const INTEGRATION: Self = Self { max_per_test_ms: 10_000, max_suite_secs: 300 };
    /// Performance-test budget: no per-test limit; 600 s total.
    pub const PERFORMANCE: Self = Self { max_per_test_ms: u64::MAX, max_suite_secs: 600 };
}

// ---------------------------------------------------------------------------
// Suite descriptor
// ---------------------------------------------------------------------------

/// A complete description of a single test suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuiteDescriptor {
    /// Human-readable suite name.
    pub name: String,
    /// Pyramid level this suite belongs to.
    pub level: PyramidLevel,
    /// Architectural layer under test.
    pub layer: TestedLayer,
    /// Timing budget for this suite.
    pub budget: TimingBudget,
    /// Coverage requirements (may be `None` for perf suites).
    pub coverage: Option<CoverageThreshold>,
    /// Whether this suite is required to pass before a merge request may be merged.
    pub required_for_merge: bool,
}

impl TestSuiteDescriptor {
    /// Create a new suite descriptor.
    pub fn new(
        name: impl Into<String>,
        level: PyramidLevel,
        layer: TestedLayer,
        budget: TimingBudget,
        coverage: Option<CoverageThreshold>,
        required_for_merge: bool,
    ) -> Self {
        Self {
            name: name.into(),
            level,
            layer,
            budget,
            coverage,
            required_for_merge,
        }
    }
}

// ---------------------------------------------------------------------------
// Strategy registry
// ---------------------------------------------------------------------------

/// The complete infinityOS test strategy: all registered suites.
///
/// Construct via [`TestStrategy::default`] to get the canonical set of suites,
/// or use the builder API to register additional suites.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestStrategy {
    suites: Vec<TestSuiteDescriptor>,
}

impl TestStrategy {
    /// Construct the canonical infinityOS test strategy with all default suites.
    pub fn canonical() -> Self {
        let mut s = Self::default();

        let line80 = CoverageThreshold::new(80.0, 70.0).expect("valid");
        let line70 = CoverageThreshold::new(70.0, 60.0).expect("valid");

        // --- Unit suites ---
        for (name, layer) in [
            ("kernel-unit", TestedLayer::Kernel),
            ("runtime-core-unit", TestedLayer::RuntimeCore),
            ("controller-unit", TestedLayer::Controller),
            ("canvas-unit", TestedLayer::Canvas),
            ("reliability-unit", TestedLayer::Reliability),
            ("security-unit", TestedLayer::Security),
        ] {
            s.register(TestSuiteDescriptor::new(
                name,
                PyramidLevel::Unit,
                layer,
                TimingBudget::UNIT,
                Some(line80),
                true,
            ));
        }

        // --- Integration suites ---
        for (name, layer) in [
            ("kernel-runtime-ffi-integration", TestedLayer::CrossLayer),
            ("actionlog-completeness-integration", TestedLayer::CrossLayer),
            ("mesh-provenance-integration", TestedLayer::CrossLayer),
        ] {
            s.register(TestSuiteDescriptor::new(
                name,
                PyramidLevel::Integration,
                layer,
                TimingBudget::INTEGRATION,
                Some(line70),
                true,
            ));
        }

        // --- Performance suites ---
        for (name, layer) in [
            ("scheduler-throughput-perf", TestedLayer::Kernel),
            ("executor-latency-perf", TestedLayer::RuntimeCore),
            ("actionlog-write-throughput-perf", TestedLayer::Controller),
            ("mesh-artifact-write-throughput-perf", TestedLayer::Controller),
        ] {
            s.register(TestSuiteDescriptor::new(
                name,
                PyramidLevel::Performance,
                layer,
                TimingBudget::PERFORMANCE,
                None,
                false,
            ));
        }

        s
    }

    /// Register an additional suite.
    pub fn register(&mut self, suite: TestSuiteDescriptor) {
        self.suites.push(suite);
    }

    /// Return all registered suites.
    pub fn suites(&self) -> &[TestSuiteDescriptor] {
        &self.suites
    }

    /// Return only the suites required for merge readiness.
    pub fn required_suites(&self) -> impl Iterator<Item = &TestSuiteDescriptor> {
        self.suites.iter().filter(|s| s.required_for_merge)
    }

    /// Look up a suite by name.
    pub fn get(&self, name: &str) -> Option<&TestSuiteDescriptor> {
        self.suites.iter().find(|s| s.name == name)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_strategy_has_required_suites() {
        let strategy = TestStrategy::canonical();
        assert!(!strategy.suites().is_empty());
        let required: Vec<_> = strategy.required_suites().collect();
        assert!(!required.is_empty());
    }

    #[test]
    fn coverage_threshold_rejects_out_of_range() {
        assert!(CoverageThreshold::new(101.0, 50.0).is_err());
        assert!(CoverageThreshold::new(50.0, -1.0).is_err());
        assert!(CoverageThreshold::new(80.0, 70.0).is_ok());
    }

    #[test]
    fn strategy_get_returns_correct_suite() {
        let strategy = TestStrategy::canonical();
        let suite = strategy.get("kernel-unit");
        assert!(suite.is_some());
        assert_eq!(suite.unwrap().layer, TestedLayer::Kernel);
        assert_eq!(suite.unwrap().level, PyramidLevel::Unit);
    }

    #[test]
    fn timing_budget_constants_are_consistent() {
        assert!(TimingBudget::UNIT.max_per_test_ms < TimingBudget::INTEGRATION.max_per_test_ms);
        assert!(TimingBudget::UNIT.max_suite_secs < TimingBudget::INTEGRATION.max_suite_secs);
    }

    #[test]
    fn strategy_serializes_and_roundtrips() {
        let strategy = TestStrategy::canonical();
        let json = serde_json::to_string(&strategy).expect("serialise");
        let back: TestStrategy = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.suites().len(), strategy.suites().len());
    }
}
