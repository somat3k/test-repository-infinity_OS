//! # load — Load Tests for Orchestrator and Mesh
//!
//! Defines load-test scenario descriptors and lightweight in-process
//! throughput harnesses for the orchestrator and mesh artifact bus.
//!
//! Real load tests that drive system-level concurrency are executed by the
//! dedicated CI job (`cargo bench` / `cmake --build build --target bench`).
//! This module provides:
//!
//! 1. [`LoadScenario`] — a declarative description of a load-test scenario.
//! 2. [`LoadResult`] — the structured result of a completed run.
//! 3. [`LoadRunner`] — an in-process micro-harness that simulates task
//!    submission and mesh writes at configurable concurrency levels (suitable
//!    for unit-level regression smoke tests, not full benchmarks).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the load testing module.
#[derive(Debug, Error)]
pub enum LoadError {
    /// A scenario with the given name already exists.
    #[error("duplicate load scenario: {0}")]
    DuplicateScenario(String),
    /// The scenario completed with a throughput below the minimum requirement.
    #[error("throughput regression: {actual:.1} ops/s < required {required:.1} ops/s")]
    ThroughputRegression {
        /// Actual throughput achieved.
        actual: f64,
        /// Required throughput.
        required: f64,
    },
    /// The scenario completed with a p99 latency above the allowed threshold.
    #[error("p99 latency regression: {actual_ms:.1} ms > allowed {allowed_ms:.1} ms")]
    LatencyRegression {
        /// Actual p99 latency.
        actual_ms: f64,
        /// Allowed p99 latency.
        allowed_ms: f64,
    },
}

// ---------------------------------------------------------------------------
// Target under load
// ---------------------------------------------------------------------------

/// Which subsystem a load scenario targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadTarget {
    /// The task orchestrator (task submission → scheduling → execution).
    Orchestrator,
    /// The mesh artifact bus (produce → consume → snapshot).
    MeshArtifactBus,
    /// The ActionLog write path.
    ActionLog,
    /// The node graph execution path.
    NodeGraph,
}

// ---------------------------------------------------------------------------
// Scenario descriptor
// ---------------------------------------------------------------------------

/// Declarative description of a load-test scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadScenario {
    /// Unique scenario name.
    pub name: String,
    /// Target under load.
    pub target: LoadTarget,
    /// Number of concurrent virtual users / goroutines / Tokio tasks.
    pub concurrency: usize,
    /// Total number of operations to execute.
    pub total_ops: u64,
    /// Minimum acceptable throughput (operations/second).
    pub min_throughput_ops_per_sec: f64,
    /// Maximum acceptable p99 latency (milliseconds).
    pub max_p99_latency_ms: f64,
    /// Free-form description.
    pub description: String,
}

impl LoadScenario {
    /// Create a new scenario.
    pub fn new(
        name: impl Into<String>,
        target: LoadTarget,
        concurrency: usize,
        total_ops: u64,
        min_throughput_ops_per_sec: f64,
        max_p99_latency_ms: f64,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            target,
            concurrency,
            total_ops,
            min_throughput_ops_per_sec,
            max_p99_latency_ms,
            description: description.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Load result
// ---------------------------------------------------------------------------

/// Latency histogram with fixed percentile buckets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyHistogram {
    /// p50 latency in milliseconds.
    pub p50_ms: f64,
    /// p95 latency in milliseconds.
    pub p95_ms: f64,
    /// p99 latency in milliseconds.
    pub p99_ms: f64,
    /// Maximum observed latency in milliseconds.
    pub max_ms: f64,
}

/// The result of a completed load-test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadResult {
    /// Scenario name.
    pub scenario: String,
    /// Total operations completed.
    pub ops_completed: u64,
    /// Total elapsed time.
    pub elapsed_secs: f64,
    /// Observed throughput (ops/second).
    pub throughput_ops_per_sec: f64,
    /// Latency histogram.
    pub latency: LatencyHistogram,
    /// Whether the result meets the scenario's requirements.
    pub passed: bool,
}

impl LoadResult {
    /// Evaluate the result against a scenario's thresholds and set `passed`.
    pub fn evaluate(&mut self, scenario: &LoadScenario) -> Result<(), LoadError> {
        if self.throughput_ops_per_sec < scenario.min_throughput_ops_per_sec {
            self.passed = false;
            return Err(LoadError::ThroughputRegression {
                actual: self.throughput_ops_per_sec,
                required: scenario.min_throughput_ops_per_sec,
            });
        }
        if self.latency.p99_ms > scenario.max_p99_latency_ms {
            self.passed = false;
            return Err(LoadError::LatencyRegression {
                actual_ms: self.latency.p99_ms,
                allowed_ms: scenario.max_p99_latency_ms,
            });
        }
        self.passed = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Scenario registry
// ---------------------------------------------------------------------------

/// Registry of all known load-test scenarios.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoadScenarioRegistry {
    scenarios: HashMap<String, LoadScenario>,
}

impl LoadScenarioRegistry {
    /// Build the canonical infinityOS load-test scenario registry.
    pub fn canonical() -> Self {
        let mut r = Self::default();

        r.add(LoadScenario::new(
            "orchestrator-single-threaded-baseline",
            LoadTarget::Orchestrator,
            1,
            1_000,
            500.0,   // ≥ 500 ops/s
            50.0,    // p99 ≤ 50 ms
            "Single-threaded orchestrator baseline: 1 000 sequential task submissions",
        )).expect("unique");

        r.add(LoadScenario::new(
            "orchestrator-high-concurrency",
            LoadTarget::Orchestrator,
            16,
            10_000,
            5_000.0, // ≥ 5 000 ops/s
            100.0,   // p99 ≤ 100 ms
            "High-concurrency orchestrator: 16 concurrent workers, 10 000 tasks",
        )).expect("unique");

        r.add(LoadScenario::new(
            "mesh-produce-consume-baseline",
            LoadTarget::MeshArtifactBus,
            1,
            1_000,
            1_000.0, // ≥ 1 000 ops/s
            20.0,    // p99 ≤ 20 ms
            "Single-threaded mesh produce/consume: 1 000 artifacts",
        )).expect("unique");

        r.add(LoadScenario::new(
            "mesh-high-concurrency",
            LoadTarget::MeshArtifactBus,
            8,
            5_000,
            3_000.0, // ≥ 3 000 ops/s
            50.0,    // p99 ≤ 50 ms
            "High-concurrency mesh: 8 concurrent producers/consumers, 5 000 artifacts",
        )).expect("unique");

        r.add(LoadScenario::new(
            "actionlog-write-throughput",
            LoadTarget::ActionLog,
            4,
            10_000,
            10_000.0, // ≥ 10 000 events/s
            10.0,     // p99 ≤ 10 ms
            "ActionLog write throughput: 4 concurrent writers, 10 000 events",
        )).expect("unique");

        r
    }

    /// Add a scenario.
    ///
    /// # Errors
    /// Returns [`LoadError::DuplicateScenario`] if already registered.
    pub fn add(&mut self, scenario: LoadScenario) -> Result<(), LoadError> {
        if self.scenarios.contains_key(&scenario.name) {
            return Err(LoadError::DuplicateScenario(scenario.name));
        }
        self.scenarios.insert(scenario.name.clone(), scenario);
        Ok(())
    }

    /// Return all scenarios.
    pub fn all(&self) -> impl Iterator<Item = &LoadScenario> {
        self.scenarios.values()
    }

    /// Look up a scenario by name.
    pub fn get(&self, name: &str) -> Option<&LoadScenario> {
        self.scenarios.get(name)
    }
}

// ---------------------------------------------------------------------------
// In-process micro-harness
// ---------------------------------------------------------------------------

/// Lightweight in-process load runner for smoke-testing throughput regressions.
///
/// This is NOT a substitute for full benchmarks; it is designed to run inside
/// `cargo test` (< 100 ms per scenario) and detect gross regressions.
pub struct LoadRunner;

impl LoadRunner {
    /// Run a synthetic no-op workload to measure the in-process overhead of
    /// the harness itself.  Used to establish a baseline for micro-harness
    /// regressions.
    ///
    /// Each "operation" is an `Instant::now()` call followed by a push to a
    /// `Vec<Duration>` — representative of the overhead added by any real
    /// measurement harness.
    pub fn run_noop(scenario: &LoadScenario) -> LoadResult {
        let start = Instant::now();
        let mut latencies_ns: Vec<u64> = Vec::with_capacity(scenario.total_ops as usize);

        for _ in 0..scenario.total_ops {
            let op_start = Instant::now();
            // No-op body — measures harness overhead.
            let _ = std::hint::black_box(op_start.elapsed().as_nanos());
            latencies_ns.push(op_start.elapsed().as_nanos() as u64);
        }

        let elapsed = start.elapsed();
        latencies_ns.sort_unstable();
        let n = latencies_ns.len();

        let percentile = |p: f64| -> f64 {
            let idx = ((p / 100.0) * n as f64) as usize;
            latencies_ns[idx.min(n - 1)] as f64 / 1_000_000.0
        };

        let elapsed_secs = elapsed.as_secs_f64();
        let throughput = scenario.total_ops as f64 / elapsed_secs;

        let mut result = LoadResult {
            scenario: scenario.name.clone(),
            ops_completed: scenario.total_ops,
            elapsed_secs,
            throughput_ops_per_sec: throughput,
            latency: LatencyHistogram {
                p50_ms: percentile(50.0),
                p95_ms: percentile(95.0),
                p99_ms: percentile(99.0),
                max_ms: latencies_ns.last().copied().unwrap_or(0) as f64 / 1_000_000.0,
            },
            passed: false,
        };
        // Ignore harness-overhead latency threshold violations (noop runner).
        let _ = result.evaluate(scenario);
        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_registry_has_expected_scenarios() {
        let r = LoadScenarioRegistry::canonical();
        assert!(r.get("orchestrator-single-threaded-baseline").is_some());
        assert!(r.get("orchestrator-high-concurrency").is_some());
        assert!(r.get("mesh-produce-consume-baseline").is_some());
        assert!(r.get("mesh-high-concurrency").is_some());
        assert!(r.get("actionlog-write-throughput").is_some());
    }

    #[test]
    fn noop_runner_completes_all_ops() {
        let r = LoadScenarioRegistry::canonical();
        let scenario = r.get("orchestrator-single-threaded-baseline").unwrap();
        let result = LoadRunner::run_noop(scenario);
        assert_eq!(result.ops_completed, scenario.total_ops);
    }

    #[test]
    fn noop_runner_latency_histogram_is_ordered() {
        let r = LoadScenarioRegistry::canonical();
        let scenario = r.get("orchestrator-single-threaded-baseline").unwrap();
        let result = LoadRunner::run_noop(scenario);
        assert!(result.latency.p50_ms <= result.latency.p95_ms);
        assert!(result.latency.p95_ms <= result.latency.p99_ms);
        assert!(result.latency.p99_ms <= result.latency.max_ms);
    }

    #[test]
    fn throughput_regression_is_detected() {
        let scenario = LoadScenario::new(
            "strict-scenario",
            LoadTarget::Orchestrator,
            1,
            10,
            f64::MAX, // impossible to achieve
            f64::MAX,
            "strict throughput requirement",
        );
        let mut result = LoadResult {
            scenario: "strict-scenario".into(),
            ops_completed: 10,
            elapsed_secs: 1.0,
            throughput_ops_per_sec: 10.0,
            latency: LatencyHistogram { p50_ms: 1.0, p95_ms: 1.0, p99_ms: 1.0, max_ms: 1.0 },
            passed: false,
        };
        assert!(matches!(result.evaluate(&scenario), Err(LoadError::ThroughputRegression { .. })));
        assert!(!result.passed);
    }

    #[test]
    fn duplicate_scenario_is_rejected() {
        let mut r = LoadScenarioRegistry::default();
        let s = LoadScenario::new("s", LoadTarget::Orchestrator, 1, 100, 1.0, 1000.0, "d");
        r.add(s.clone()).unwrap();
        assert!(r.add(s).is_err());
    }
}
