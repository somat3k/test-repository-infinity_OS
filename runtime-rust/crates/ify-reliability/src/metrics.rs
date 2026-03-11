//! Reliability metrics — MTTR, error budget, and regression rate tracking.
//!
//! This module implements Epic K item 2: *Track MTTR/error budget/regression
//! rate metrics*, and provides the data structures consumed by the
//! [`crate::dashboard`] and [`crate::incident`] modules.
//!
//! # Metric definitions
//!
//! | Metric | Unit | Description |
//! |--------|------|-------------|
//! | MTTR | seconds | Mean Time To Recovery: average time from incident open to resolved. |
//! | Error budget | ratio 0.0–1.0 | Fraction of the SLO error budget remaining in the rolling window. |
//! | Regression rate | events/cycle | Number of regressions detected in the current Kaizen cycle. |
//! | Availability | ratio 0.0–1.0 | Fraction of time the system was within SLO. |

use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the metrics subsystem.
#[derive(Debug, Error)]
pub enum MetricsError {
    /// An incident duration was invalid (negative or overflow).
    #[error("invalid incident duration: {0:?}")]
    InvalidDuration(Duration),
    /// The rolling window is empty — a metric cannot be computed.
    #[error("no samples in the rolling window for metric '{0}'")]
    EmptyWindow(String),
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// IncidentRecord
// ---------------------------------------------------------------------------

/// A single resolved or ongoing incident record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentRecord {
    /// Unique incident identifier.
    pub id: String,
    /// Unix timestamp (seconds) when the incident was opened.
    pub opened_at: u64,
    /// Unix timestamp (seconds) when the incident was resolved, or `None` if still open.
    pub resolved_at: Option<u64>,
    /// Human-readable title.
    pub title: String,
    /// Severity label.
    pub severity: IncidentSeverity,
}

/// Incident severity classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IncidentSeverity {
    /// Critical — immediate action required; SLO breached.
    Critical,
    /// High — SLO at risk within the error budget window.
    High,
    /// Medium — degraded behaviour; within error budget.
    Medium,
    /// Low — informational; no user impact expected.
    Low,
}

impl IncidentRecord {
    /// Create a new open incident.
    pub fn open(id: impl Into<String>, title: impl Into<String>, severity: IncidentSeverity) -> Self {
        Self {
            id: id.into(),
            opened_at: now_secs(),
            resolved_at: None,
            title: title.into(),
            severity,
        }
    }

    /// Mark the incident resolved now.
    pub fn resolve(&mut self) {
        if self.resolved_at.is_none() {
            self.resolved_at = Some(now_secs());
        }
    }

    /// Time-to-recovery for resolved incidents (seconds).
    pub fn ttr_secs(&self) -> Option<u64> {
        self.resolved_at.map(|r| r.saturating_sub(self.opened_at))
    }
}

// ---------------------------------------------------------------------------
// MTTR tracker
// ---------------------------------------------------------------------------

/// Tracks mean-time-to-recovery over a rolling window.
///
/// Only resolved incidents contribute to the MTTR calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MttrTracker {
    records: VecDeque<IncidentRecord>,
    window_size: usize,
}

impl MttrTracker {
    /// Create a tracker with the given rolling-window capacity.
    pub fn new(window_size: usize) -> Self {
        Self {
            records: VecDeque::new(),
            window_size: window_size.max(1),
        }
    }

    /// Record an incident (open or resolved).
    pub fn push(&mut self, record: IncidentRecord) {
        if self.records.len() >= self.window_size {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }

    /// Compute the current MTTR in seconds, or an error if no resolved incidents exist.
    pub fn mttr_secs(&self) -> Result<f64, MetricsError> {
        let resolved: Vec<u64> = self
            .records
            .iter()
            .filter_map(|r| r.ttr_secs())
            .collect();
        if resolved.is_empty() {
            return Err(MetricsError::EmptyWindow("MTTR".into()));
        }
        let sum: u64 = resolved.iter().sum();
        Ok(sum as f64 / resolved.len() as f64)
    }

    /// Number of incidents in the window (open + resolved).
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns `true` if no incidents have been recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Iterate over all records in the window.
    pub fn iter(&self) -> impl Iterator<Item = &IncidentRecord> {
        self.records.iter()
    }
}

// ---------------------------------------------------------------------------
// ErrorBudget
// ---------------------------------------------------------------------------

/// Tracks the remaining error budget for a single SLO.
///
/// The budget is modelled as a fraction of allowed downtime in a fixed window.
/// Each call to [`ErrorBudget::consume`] reduces the available budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBudget {
    /// The target availability expressed as a ratio (e.g. 0.999 for 99.9 %).
    pub target: f64,
    /// Total window duration in seconds (default: 30 days = 2_592_000 s).
    pub window_secs: u64,
    /// Seconds of downtime consumed in the current window.
    consumed_secs: f64,
}

impl ErrorBudget {
    /// Create a new error budget.
    ///
    /// `target` is the SLO availability (0.0–1.0), e.g. `0.999`.
    /// `window_secs` is the rolling window length in seconds.
    pub fn new(target: f64, window_secs: u64) -> Self {
        Self {
            target: target.clamp(0.0, 1.0),
            window_secs,
            consumed_secs: 0.0,
        }
    }

    /// Total allowed downtime in the window (seconds).
    pub fn allowed_downtime_secs(&self) -> f64 {
        (1.0 - self.target) * self.window_secs as f64
    }

    /// Consume `secs` seconds of downtime from the budget.
    pub fn consume(&mut self, secs: f64) {
        self.consumed_secs += secs.max(0.0);
        info!(
            consumed_secs = self.consumed_secs,
            allowed = self.allowed_downtime_secs(),
            remaining_ratio = self.remaining_ratio(),
            "error_budget.consumed"
        );
    }

    /// Remaining error budget as a ratio of the total allowed downtime (0.0–1.0).
    ///
    /// Returns 0.0 when the budget is exhausted or overrun.
    pub fn remaining_ratio(&self) -> f64 {
        let allowed = self.allowed_downtime_secs();
        if allowed <= 0.0 {
            return 0.0;
        }
        ((allowed - self.consumed_secs) / allowed).clamp(0.0, 1.0)
    }

    /// Returns `true` when the error budget has been exhausted.
    pub fn is_exhausted(&self) -> bool {
        self.consumed_secs >= self.allowed_downtime_secs()
    }

    /// Reset the error budget for a new window.
    pub fn reset(&mut self) {
        self.consumed_secs = 0.0;
    }
}

// ---------------------------------------------------------------------------
// RegressionTracker
// ---------------------------------------------------------------------------

/// Tracks regression counts over Kaizen cycles.
///
/// A *regression* is a measurable deterioration in a tracked metric relative
/// to the established baseline for that cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionTracker {
    /// Regression events indexed by cycle identifier.
    cycles: Vec<CycleEntry>,
    /// Maximum cycles retained.
    window_size: usize,
}

/// One Kaizen cycle regression summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleEntry {
    /// Human-readable cycle identifier (e.g. "2026-W11").
    pub cycle_id: String,
    /// Number of regressions detected this cycle.
    pub regression_count: u32,
    /// Number of regressions that were resolved this cycle.
    pub resolved_count: u32,
    /// Unix timestamp when the cycle was recorded.
    pub recorded_at: u64,
}

impl RegressionTracker {
    /// Create a tracker with the given rolling-window capacity.
    pub fn new(window_size: usize) -> Self {
        Self {
            cycles: Vec::new(),
            window_size: window_size.max(1),
        }
    }

    /// Record cycle results. Pushes a new entry or updates an existing one.
    pub fn record_cycle(
        &mut self,
        cycle_id: impl Into<String>,
        regression_count: u32,
        resolved_count: u32,
    ) {
        let id = cycle_id.into();
        // Update existing entry if cycle already recorded.
        if let Some(entry) = self.cycles.iter_mut().find(|e| e.cycle_id == id) {
            entry.regression_count = regression_count;
            entry.resolved_count = resolved_count;
            entry.recorded_at = now_secs();
            return;
        }
        if self.cycles.len() >= self.window_size {
            self.cycles.remove(0);
        }
        self.cycles.push(CycleEntry {
            cycle_id: id,
            regression_count,
            resolved_count,
            recorded_at: now_secs(),
        });
    }

    /// Average regression rate (regressions per cycle) over the window.
    pub fn average_rate(&self) -> Result<f64, MetricsError> {
        if self.cycles.is_empty() {
            return Err(MetricsError::EmptyWindow("regression_rate".into()));
        }
        let total: u32 = self.cycles.iter().map(|e| e.regression_count).sum();
        Ok(total as f64 / self.cycles.len() as f64)
    }

    /// Iterate over cycle entries.
    pub fn iter(&self) -> impl Iterator<Item = &CycleEntry> {
        self.cycles.iter()
    }
}

// ---------------------------------------------------------------------------
// ReliabilityMetrics — aggregate store
// ---------------------------------------------------------------------------

/// Aggregate reliability metrics store for a single dimension.
///
/// Combines MTTR, error budget, and regression tracking behind one API,
/// used by the dashboard and incident pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReliabilityMetrics {
    /// MTTR rolling tracker.
    pub mttr: MttrTracker,
    /// Error budget for task execution SLO.
    pub task_error_budget: ErrorBudget,
    /// Error budget for UI responsiveness SLO.
    pub ui_error_budget: ErrorBudget,
    /// Regression rate tracker.
    pub regressions: RegressionTracker,
}

impl ReliabilityMetrics {
    /// Create metrics with default SLO targets (99.9 % task, 99.5 % UI) and
    /// a 30-day window.
    pub fn new() -> Self {
        const WINDOW_30D: u64 = 30 * 24 * 3600;
        Self {
            mttr: MttrTracker::new(100),
            task_error_budget: ErrorBudget::new(0.999, WINDOW_30D),
            ui_error_budget: ErrorBudget::new(0.995, WINDOW_30D),
            regressions: RegressionTracker::new(52), // ~1 year of weekly cycles
        }
    }

    /// Snapshot the metrics to a JSON value.
    pub fn snapshot(&self) -> serde_json::Value {
        let mttr = self.mttr.mttr_secs().ok();
        serde_json::json!({
            "mttr_secs": mttr,
            "task_error_budget_remaining": self.task_error_budget.remaining_ratio(),
            "ui_error_budget_remaining": self.ui_error_budget.remaining_ratio(),
            "regression_avg_rate": self.regressions.average_rate().ok(),
        })
    }
}

impl Default for ReliabilityMetrics {
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

    #[test]
    fn mttr_empty_returns_error() {
        let tracker = MttrTracker::new(10);
        assert!(tracker.mttr_secs().is_err());
    }

    #[test]
    fn mttr_single_resolved() {
        let mut tracker = MttrTracker::new(10);
        let mut rec = IncidentRecord::open("inc-1", "test", IncidentSeverity::High);
        rec.resolved_at = Some(rec.opened_at + 120); // 2 min TTR
        tracker.push(rec);
        assert!((tracker.mttr_secs().unwrap() - 120.0).abs() < 1.0);
    }

    #[test]
    fn mttr_averages_correctly() {
        let mut tracker = MttrTracker::new(10);
        for ttr in [60u64, 120, 180] {
            let mut r = IncidentRecord::open("x", "t", IncidentSeverity::Low);
            r.resolved_at = Some(r.opened_at + ttr);
            tracker.push(r);
        }
        let mttr = tracker.mttr_secs().unwrap();
        assert!((mttr - 120.0).abs() < 1.0);
    }

    #[test]
    fn error_budget_remaining_full_initially() {
        let budget = ErrorBudget::new(0.999, 30 * 24 * 3600);
        assert!((budget.remaining_ratio() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn error_budget_exhaustion() {
        let mut budget = ErrorBudget::new(0.999, 30 * 24 * 3600);
        let allowed = budget.allowed_downtime_secs();
        budget.consume(allowed);
        assert!(budget.is_exhausted());
        assert_eq!(budget.remaining_ratio(), 0.0);
    }

    #[test]
    fn error_budget_partial_consumption() {
        let mut budget = ErrorBudget::new(0.999, 30 * 24 * 3600);
        let allowed = budget.allowed_downtime_secs();
        budget.consume(allowed / 2.0);
        let ratio = budget.remaining_ratio();
        assert!((ratio - 0.5).abs() < 0.001);
    }

    #[test]
    fn regression_tracker_average_rate() {
        let mut tracker = RegressionTracker::new(10);
        tracker.record_cycle("W01", 3, 2);
        tracker.record_cycle("W02", 1, 1);
        let rate = tracker.average_rate().unwrap();
        assert!((rate - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn regression_tracker_update_existing_cycle() {
        let mut tracker = RegressionTracker::new(10);
        tracker.record_cycle("W01", 3, 2);
        tracker.record_cycle("W01", 5, 4); // update same cycle
        assert_eq!(tracker.cycles.len(), 1);
        assert_eq!(tracker.cycles[0].regression_count, 5);
    }

    #[test]
    fn metrics_snapshot_keys_present() {
        let metrics = ReliabilityMetrics::new();
        let snap = metrics.snapshot();
        assert!(snap.get("task_error_budget_remaining").is_some());
        assert!(snap.get("ui_error_budget_remaining").is_some());
        assert!(snap.get("regression_avg_rate").is_some());
    }
}
