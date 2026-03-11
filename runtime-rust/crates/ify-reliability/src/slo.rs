//! Service Level Objectives (SLOs) — Epic K item 4.
//!
//! Defines and evaluates SLOs for:
//! - **Task execution**: latency p50/p99, queue wait time, and availability.
//! - **UI responsiveness**: frame budget compliance ratio and interaction latency.
//!
//! Each SLO is a named threshold with an evaluation function. The
//! [`SloRegistry`] maintains the collection of registered SLOs and evaluates
//! them against incoming samples to produce [`SloStatus`] reports consumed by
//! the dashboard and incident pipeline.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the SLO subsystem.
#[derive(Debug, Error)]
pub enum SloError {
    /// An SLO with the given name is already registered.
    #[error("SLO '{0}' is already registered")]
    Duplicate(String),
    /// No SLO found with the given name.
    #[error("SLO '{0}' not found")]
    NotFound(String),
    /// The sample window is empty — SLO cannot be evaluated.
    #[error("no samples for SLO '{0}'")]
    EmptySamples(String),
}

// ---------------------------------------------------------------------------
// SloKind
// ---------------------------------------------------------------------------

/// Identifies the aspect of the system the SLO measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SloKind {
    /// Task execution latency (milliseconds).
    TaskLatency,
    /// Task queue wait time (milliseconds).
    TaskQueueWait,
    /// Task execution availability (ratio 0.0–1.0).
    TaskAvailability,
    /// UI frame render time (milliseconds).
    UiFrameTime,
    /// UI interaction response latency (milliseconds).
    UiInteractionLatency,
}

// ---------------------------------------------------------------------------
// SloThreshold
// ---------------------------------------------------------------------------

/// The evaluation rule for a single SLO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloThreshold {
    /// Human-readable SLO name (e.g. `"task.p99_latency_ms"`).
    pub name: String,
    /// The aspect of the system being measured.
    pub kind: SloKind,
    /// Threshold value (interpretation depends on `kind`).
    pub threshold: f64,
    /// Percentile used when evaluating latency/frame-time SLOs (0–100).
    /// Ignored for availability SLOs.
    pub percentile: u8,
    /// The minimum fraction of samples that must meet the threshold for the
    /// SLO to pass (e.g. 0.99 means 99 % of samples must be within threshold).
    pub target_ratio: f64,
}

impl SloThreshold {
    /// Create a latency SLO: at most `threshold_ms` at the given percentile.
    pub fn latency(
        name: impl Into<String>,
        kind: SloKind,
        percentile: u8,
        threshold_ms: f64,
        target_ratio: f64,
    ) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&target_ratio),
            "target_ratio {target_ratio} is out of range [0.0, 1.0]; it will be clamped"
        );
        Self {
            name: name.into(),
            kind,
            threshold: threshold_ms,
            percentile,
            target_ratio: target_ratio.clamp(0.0, 1.0),
        }
    }

    /// Create an availability SLO: at least `target_ratio` of the time.
    pub fn availability(name: impl Into<String>, kind: SloKind, target_ratio: f64) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&target_ratio),
            "target_ratio {target_ratio} is out of range [0.0, 1.0]; it will be clamped"
        );
        Self {
            name: name.into(),
            kind,
            threshold: target_ratio.clamp(0.0, 1.0),
            percentile: 0,
            target_ratio: target_ratio.clamp(0.0, 1.0),
        }
    }
}

// ---------------------------------------------------------------------------
// SloSample
// ---------------------------------------------------------------------------

/// A single measurement sample to be evaluated against an SLO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloSample {
    /// The SLO name this sample belongs to.
    pub slo_name: String,
    /// Measured value (ms for latency, ratio for availability).
    pub value: f64,
    /// Unix timestamp (seconds) of the measurement.
    pub timestamp_secs: u64,
}

// ---------------------------------------------------------------------------
// SloStatus
// ---------------------------------------------------------------------------

/// Evaluation result for a single SLO at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloStatus {
    /// SLO name.
    pub name: String,
    /// Whether the SLO is currently passing.
    pub passing: bool,
    /// The measured value at the target percentile (or availability ratio).
    pub measured_value: f64,
    /// The threshold.
    pub threshold: f64,
    /// Fraction of samples that met the threshold.
    pub compliance_ratio: f64,
    /// Number of samples evaluated.
    pub sample_count: usize,
}

// ---------------------------------------------------------------------------
// SloRegistry
// ---------------------------------------------------------------------------

/// Registry that holds SLO definitions and sample windows, and evaluates them.
#[derive(Debug)]
pub struct SloRegistry {
    thresholds: HashMap<String, SloThreshold>,
    /// Sliding sample windows keyed by SLO name.
    windows: HashMap<String, VecDeque<f64>>,
    /// Maximum samples retained per SLO.
    window_size: usize,
}

impl SloRegistry {
    /// Create a registry with the specified per-SLO sample window size.
    pub fn new(window_size: usize) -> Self {
        Self {
            thresholds: HashMap::new(),
            windows: HashMap::new(),
            window_size: window_size.max(1),
        }
    }

    /// Create a registry pre-loaded with the default infinityOS SLOs.
    ///
    /// | SLO name | Kind | Target |
    /// |----------|------|--------|
    /// | `task.p50_latency_ms` | TaskLatency | p50 ≤ 200 ms |
    /// | `task.p99_latency_ms` | TaskLatency | p99 ≤ 2000 ms |
    /// | `task.availability` | TaskAvailability | ≥ 99.9 % |
    /// | `ui.frame_time_ms` | UiFrameTime | p99 ≤ 16 ms |
    /// | `ui.interaction_latency_ms` | UiInteractionLatency | p99 ≤ 100 ms |
    pub fn with_defaults() -> Self {
        let mut reg = Self::new(1000);
        let defaults = vec![
            SloThreshold::latency(
                "task.p50_latency_ms",
                SloKind::TaskLatency,
                50,
                200.0,
                0.999,
            ),
            SloThreshold::latency(
                "task.p99_latency_ms",
                SloKind::TaskLatency,
                99,
                2000.0,
                0.999,
            ),
            SloThreshold::availability(
                "task.availability",
                SloKind::TaskAvailability,
                0.999,
            ),
            SloThreshold::latency(
                "ui.frame_time_ms",
                SloKind::UiFrameTime,
                99,
                16.0,
                0.999,
            ),
            SloThreshold::latency(
                "ui.interaction_latency_ms",
                SloKind::UiInteractionLatency,
                99,
                100.0,
                0.999,
            ),
        ];
        for slo in defaults {
            // Ignore duplicates at construction.
            let _ = reg.register(slo);
        }
        reg
    }

    /// Register a new SLO.
    pub fn register(&mut self, slo: SloThreshold) -> Result<(), SloError> {
        if self.thresholds.contains_key(&slo.name) {
            return Err(SloError::Duplicate(slo.name.clone()));
        }
        self.windows
            .insert(slo.name.clone(), VecDeque::new());
        self.thresholds.insert(slo.name.clone(), slo);
        Ok(())
    }

    /// Record a sample for the named SLO.
    pub fn record(&mut self, sample: SloSample) -> Result<(), SloError> {
        let window = self
            .windows
            .get_mut(&sample.slo_name)
            .ok_or_else(|| SloError::NotFound(sample.slo_name.clone()))?;
        if window.len() >= self.window_size {
            window.pop_front();
        }
        window.push_back(sample.value);
        Ok(())
    }

    /// Evaluate the named SLO and return its current status.
    pub fn evaluate(&self, name: &str) -> Result<SloStatus, SloError> {
        let threshold = self
            .thresholds
            .get(name)
            .ok_or_else(|| SloError::NotFound(name.to_string()))?;
        let window = self
            .windows
            .get(name)
            .ok_or_else(|| SloError::NotFound(name.to_string()))?;
        if window.is_empty() {
            return Err(SloError::EmptySamples(name.to_string()));
        }

        let mut sorted: Vec<f64> = window.iter().copied().collect();
        sorted.sort_by(|a, b| a.total_cmp(b));

        let measured = if threshold.kind == SloKind::TaskAvailability {
            // For availability, the "value" is already a ratio; take the mean.
            sorted.iter().sum::<f64>() / sorted.len() as f64
        } else {
            let idx = ((threshold.percentile as f64 / 100.0) * (sorted.len() - 1) as f64)
                .round() as usize;
            sorted[idx.min(sorted.len() - 1)]
        };

        let compliant = sorted
            .iter()
            .filter(|&&v| {
                if threshold.kind == SloKind::TaskAvailability {
                    v >= threshold.threshold
                } else {
                    v <= threshold.threshold
                }
            })
            .count();
        let compliance_ratio = compliant as f64 / sorted.len() as f64;
        let passing = compliance_ratio >= threshold.target_ratio;

        if !passing {
            warn!(
                slo = name,
                measured,
                threshold = threshold.threshold,
                compliance_ratio,
                "slo.breach_detected"
            );
        }

        Ok(SloStatus {
            name: name.to_string(),
            passing,
            measured_value: measured,
            threshold: threshold.threshold,
            compliance_ratio,
            sample_count: window.len(),
        })
    }

    /// Evaluate all registered SLOs and return a map of statuses.
    pub fn evaluate_all(&self) -> HashMap<String, Result<SloStatus, SloError>> {
        self.thresholds
            .keys()
            .map(|name| (name.clone(), self.evaluate(name)))
            .collect()
    }

    /// Returns the names of all registered SLOs.
    pub fn slo_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.thresholds.keys().cloned().collect();
        names.sort();
        names
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample(name: &str, value: f64) -> SloSample {
        SloSample {
            slo_name: name.to_string(),
            value,
            timestamp_secs: 0,
        }
    }

    #[test]
    fn default_slos_registered() {
        let reg = SloRegistry::with_defaults();
        assert!(reg.slo_names().contains(&"task.p99_latency_ms".to_string()));
        assert!(reg.slo_names().contains(&"ui.frame_time_ms".to_string()));
    }

    #[test]
    fn duplicate_registration_fails() {
        let mut reg = SloRegistry::new(100);
        let slo = SloThreshold::latency("foo", SloKind::TaskLatency, 99, 200.0, 0.99);
        reg.register(slo.clone()).unwrap();
        let slo2 = SloThreshold::latency("foo", SloKind::TaskLatency, 99, 200.0, 0.99);
        assert!(reg.register(slo2).is_err());
    }

    #[test]
    fn slo_passes_within_threshold() {
        let mut reg = SloRegistry::new(100);
        reg.register(SloThreshold::latency(
            "task.p99_latency_ms",
            SloKind::TaskLatency,
            99,
            2000.0,
            0.999,
        ))
        .unwrap();
        for _ in 0..100 {
            reg.record(make_sample("task.p99_latency_ms", 100.0))
                .unwrap();
        }
        let status = reg.evaluate("task.p99_latency_ms").unwrap();
        assert!(status.passing);
    }

    #[test]
    fn slo_breaches_above_threshold() {
        let mut reg = SloRegistry::new(100);
        reg.register(SloThreshold::latency(
            "task.p99_latency_ms",
            SloKind::TaskLatency,
            99,
            200.0,
            0.999,
        ))
        .unwrap();
        // 50 % of samples above threshold → compliance 0.5 < 0.999
        for _ in 0..50 {
            reg.record(make_sample("task.p99_latency_ms", 100.0))
                .unwrap();
        }
        for _ in 0..50 {
            reg.record(make_sample("task.p99_latency_ms", 999.0))
                .unwrap();
        }
        let status = reg.evaluate("task.p99_latency_ms").unwrap();
        assert!(!status.passing);
    }

    #[test]
    fn empty_window_returns_error() {
        let mut reg = SloRegistry::new(100);
        reg.register(SloThreshold::latency(
            "task.p50_latency_ms",
            SloKind::TaskLatency,
            50,
            200.0,
            0.999,
        ))
        .unwrap();
        assert!(reg.evaluate("task.p50_latency_ms").is_err());
    }

    #[test]
    fn evaluate_all_returns_all_slos() {
        let mut reg = SloRegistry::with_defaults();
        // Feed one sample into each SLO so they can be evaluated.
        for name in reg.slo_names() {
            reg.record(make_sample(&name, 1.0)).unwrap();
        }
        let results = reg.evaluate_all();
        assert_eq!(results.len(), 5);
    }
}
