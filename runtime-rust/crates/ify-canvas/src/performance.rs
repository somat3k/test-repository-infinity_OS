//! # performance — Canvas Performance Budgets (FPS + Large-Graph Handling)
//!
//! Defines and enforces performance budgets for the infinity canvas:
//! target frame times, maximum visible node counts per zoom tier, render
//! batch sizes, and adaptive culling thresholds.
//!
//! ## Budgets
//!
//! | Metric                      | Budget         |
//! |-----------------------------|----------------|
//! | Target FPS                  | ≥ 60 fps       |
//! | Frame budget                | ≤ 16 ms        |
//! | Max visible nodes (Standard)| 500            |
//! | Max visible nodes (Overview)| 2 000          |
//! | Max visible nodes (Galaxy)  | 10 000 badges  |
//! | Render batch size           | 100 nodes      |
//! | Zoom transition budget      | ≤ 16 ms        |

use serde::{Deserialize, Serialize};

use crate::zoom::ZoomLevel;

// ---------------------------------------------------------------------------
// PerformanceBudget
// ---------------------------------------------------------------------------

/// Performance budget constants for the canvas renderer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PerformanceBudget {
    /// Target frame duration in milliseconds.
    pub frame_budget_ms: f64,
    /// Maximum number of fully-rendered nodes per frame.
    pub max_visible_nodes: u32,
    /// Preferred node batch size for incremental rendering.
    pub render_batch_size: u32,
    /// Maximum milliseconds allowed for a zoom-level transition.
    pub zoom_transition_max_ms: f64,
}

impl PerformanceBudget {
    /// Return the appropriate budget for the given zoom level.
    pub fn for_zoom(zoom: ZoomLevel) -> Self {
        match zoom {
            ZoomLevel::Galaxy => Self {
                frame_budget_ms: 16.0,
                max_visible_nodes: 10_000,
                render_batch_size: 500,
                zoom_transition_max_ms: 16.0,
            },
            ZoomLevel::Constellation => Self {
                frame_budget_ms: 16.0,
                max_visible_nodes: 5_000,
                render_batch_size: 250,
                zoom_transition_max_ms: 16.0,
            },
            ZoomLevel::Overview => Self {
                frame_budget_ms: 16.0,
                max_visible_nodes: 2_000,
                render_batch_size: 100,
                zoom_transition_max_ms: 16.0,
            },
            ZoomLevel::Standard => Self {
                frame_budget_ms: 16.0,
                max_visible_nodes: 500,
                render_batch_size: 50,
                zoom_transition_max_ms: 16.0,
            },
            ZoomLevel::Micro => Self {
                frame_budget_ms: 16.0,
                max_visible_nodes: 100,
                render_batch_size: 20,
                zoom_transition_max_ms: 16.0,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// FrameSample
// ---------------------------------------------------------------------------

/// A single frame timing sample.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameSample {
    /// Monotonic frame index.
    pub frame_index: u64,
    /// Time taken to render the frame in milliseconds.
    pub duration_ms: f64,
    /// Number of nodes that were rendered this frame.
    pub nodes_rendered: u32,
}

impl FrameSample {
    /// Returns `true` if the frame was rendered within the given budget.
    pub fn within_budget(self, budget: &PerformanceBudget) -> bool {
        self.duration_ms <= budget.frame_budget_ms
    }
}

// ---------------------------------------------------------------------------
// PerformanceMonitor
// ---------------------------------------------------------------------------

/// Accumulates frame timing samples and computes live metrics.
///
/// A ring buffer of the last `N` frames is kept in memory to compute a
/// rolling average frame time and track budget violations.
#[derive(Debug)]
pub struct PerformanceMonitor {
    budget: PerformanceBudget,
    samples: std::collections::VecDeque<FrameSample>,
    window: usize,
    total_frames: u64,
    budget_violations: u64,
}

impl PerformanceMonitor {
    /// Create a monitor with the given budget and rolling window size.
    pub fn new(budget: PerformanceBudget, window: usize) -> Self {
        Self {
            budget,
            samples: std::collections::VecDeque::with_capacity(window),
            window,
            total_frames: 0,
            budget_violations: 0,
        }
    }

    /// Record a frame sample.
    pub fn record(&mut self, sample: FrameSample) {
        if !sample.within_budget(&self.budget) {
            self.budget_violations += 1;
        }
        self.total_frames += 1;
        if self.samples.len() == self.window {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Rolling average frame duration in milliseconds.
    pub fn avg_frame_ms(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        self.samples.iter().map(|s| s.duration_ms).sum::<f64>() / self.samples.len() as f64
    }

    /// Current rolling FPS estimate.
    pub fn fps(&self) -> f64 {
        let avg = self.avg_frame_ms();
        if avg <= 0.0 {
            return 0.0;
        }
        1000.0 / avg
    }

    /// Total number of budget violations recorded since creation.
    pub fn budget_violations(&self) -> u64 {
        self.budget_violations
    }

    /// Total frames recorded since creation.
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Returns `true` if the current rolling average is within the budget.
    pub fn is_within_budget(&self) -> bool {
        self.avg_frame_ms() <= self.budget.frame_budget_ms
    }

    /// Returns a reference to the current performance budget.
    pub fn budget(&self) -> &PerformanceBudget {
        &self.budget
    }

    /// Update the budget (e.g., when the zoom level changes).
    pub fn set_budget(&mut self, budget: PerformanceBudget) {
        self.budget = budget;
    }
}

// ---------------------------------------------------------------------------
// AdaptiveCuller
// ---------------------------------------------------------------------------

/// Adaptive culling reduces the number of rendered nodes when the frame
/// budget is being exceeded, restoring detail when headroom is available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveCuller {
    /// Current maximum node count being enforced.
    pub current_max: u32,
    /// The hard budget maximum.
    pub budget_max: u32,
    /// Step size used when increasing or decreasing the limit.
    pub step: u32,
}

impl AdaptiveCuller {
    /// Create a culler initialised to the budget's max visible count.
    pub fn from_budget(budget: &PerformanceBudget) -> Self {
        Self {
            current_max: budget.max_visible_nodes,
            budget_max: budget.max_visible_nodes,
            step: (budget.max_visible_nodes / 10).max(1),
        }
    }

    /// Called each frame: tightens the limit if over budget, relaxes if under.
    pub fn adjust(&mut self, over_budget: bool) {
        if over_budget {
            self.current_max = self.current_max.saturating_sub(self.step);
        } else {
            self.current_max = (self.current_max + self.step).min(self.budget_max);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_all_target_60fps() {
        for level in ZoomLevel::all() {
            let b = PerformanceBudget::for_zoom(level);
            assert!(
                b.frame_budget_ms <= 16.0 + f64::EPSILON,
                "{level} frame budget exceeds 16 ms"
            );
        }
    }

    #[test]
    fn frame_sample_within_budget() {
        let budget = PerformanceBudget::for_zoom(ZoomLevel::Standard);
        let ok = FrameSample { frame_index: 0, duration_ms: 12.0, nodes_rendered: 50 };
        let over = FrameSample { frame_index: 1, duration_ms: 20.0, nodes_rendered: 50 };
        assert!(ok.within_budget(&budget));
        assert!(!over.within_budget(&budget));
    }

    #[test]
    fn monitor_avg_fps() {
        let budget = PerformanceBudget::for_zoom(ZoomLevel::Standard);
        let mut mon = PerformanceMonitor::new(budget, 10);
        for i in 0..10 {
            mon.record(FrameSample { frame_index: i, duration_ms: 10.0, nodes_rendered: 100 });
        }
        assert!((mon.avg_frame_ms() - 10.0).abs() < 1e-9);
        assert!((mon.fps() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn monitor_budget_violations() {
        let budget = PerformanceBudget::for_zoom(ZoomLevel::Standard);
        let mut mon = PerformanceMonitor::new(budget, 10);
        mon.record(FrameSample { frame_index: 0, duration_ms: 10.0, nodes_rendered: 10 });
        mon.record(FrameSample { frame_index: 1, duration_ms: 20.0, nodes_rendered: 10 });
        assert_eq!(mon.budget_violations(), 1);
    }

    #[test]
    fn adaptive_culler_tightens_on_overrun() {
        let budget = PerformanceBudget::for_zoom(ZoomLevel::Standard);
        let mut culler = AdaptiveCuller::from_budget(&budget);
        let initial = culler.current_max;
        culler.adjust(true);
        assert!(culler.current_max < initial);
    }

    #[test]
    fn adaptive_culler_relaxes_when_headroom() {
        let budget = PerformanceBudget::for_zoom(ZoomLevel::Standard);
        let mut culler = AdaptiveCuller::from_budget(&budget);
        // Tighten first.
        culler.adjust(true);
        let tightened = culler.current_max;
        culler.adjust(false);
        assert!(culler.current_max > tightened);
    }

    #[test]
    fn adaptive_culler_does_not_exceed_budget_max() {
        let budget = PerformanceBudget::for_zoom(ZoomLevel::Standard);
        let mut culler = AdaptiveCuller::from_budget(&budget);
        for _ in 0..100 {
            culler.adjust(false);
        }
        assert_eq!(culler.current_max, culler.budget_max);
    }
}
