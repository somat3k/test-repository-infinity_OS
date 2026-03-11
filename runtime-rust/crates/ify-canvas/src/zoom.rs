//! # zoom — Zoom-Level Interaction Contracts and Limits
//!
//! Defines the five-tier infinity zoom model for the canvas, with concrete
//! thresholds, interaction limits at each level, transition rules, and the
//! public API used by the canvas renderer and input handler.
//!
//! ## Zoom levels (coarsest → finest)
//!
//! | Level | Name        | Scale range   | What is rendered           |
//! |-------|-------------|---------------|----------------------------|
//! | 5     | Galaxy      | 0.01 – 0.05   | Dimension clusters only    |
//! | 4     | Constellation | 0.05 – 0.20 | Group outlines + labels    |
//! | 3     | Overview    | 0.20 – 0.60   | Node chips with icons      |
//! | 2     | Standard    | 0.60 – 2.00   | Full node cards + ports    |
//! | 1     | Micro       | 2.00 – 8.00   | Node internals + debug data|
//!
//! Transitions between levels must complete within 16 ms to maintain ≥60 FPS.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// ZoomLevel
// ---------------------------------------------------------------------------

/// The five-tier zoom level for the infinity canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ZoomLevel {
    /// Finest granularity – node internals, debug overlays.
    Micro = 1,
    /// Default editing view – full node cards and port labels.
    Standard = 2,
    /// Graph overview – node chips with icon badges.
    Overview = 3,
    /// Group labels and outlines only.
    Constellation = 4,
    /// Coarsest view – dimension clusters.
    Galaxy = 5,
}

impl ZoomLevel {
    /// Inclusive scale range `[min, max)` for this level.
    pub fn scale_range(self) -> (f64, f64) {
        match self {
            ZoomLevel::Galaxy => (0.01, 0.05),
            ZoomLevel::Constellation => (0.05, 0.20),
            ZoomLevel::Overview => (0.20, 0.60),
            ZoomLevel::Standard => (0.60, 2.00),
            ZoomLevel::Micro => (2.00, 8.00),
        }
    }

    /// Human-readable name.
    pub fn name(self) -> &'static str {
        match self {
            ZoomLevel::Galaxy => "Galaxy",
            ZoomLevel::Constellation => "Constellation",
            ZoomLevel::Overview => "Overview",
            ZoomLevel::Standard => "Standard",
            ZoomLevel::Micro => "Micro",
        }
    }

    /// Determine the zoom level for the given absolute scale factor.
    ///
    /// # Errors
    ///
    /// Returns [`ZoomError::ScaleOutOfBounds`] if `scale` is outside the
    /// supported range `[0.01, 8.00]`.
    pub fn from_scale(scale: f64) -> Result<Self, ZoomError> {
        for level in [
            ZoomLevel::Micro,
            ZoomLevel::Standard,
            ZoomLevel::Overview,
            ZoomLevel::Constellation,
            ZoomLevel::Galaxy,
        ] {
            let (lo, hi) = level.scale_range();
            if scale >= lo && scale < hi {
                return Ok(level);
            }
        }
        // Accept the exact upper bound as the finest level.
        if scale == SCALE_MAX {
            return Ok(ZoomLevel::Micro);
        }
        Err(ZoomError::ScaleOutOfBounds { scale })
    }

    /// All zoom levels ordered coarsest-first.
    pub fn all() -> [ZoomLevel; 5] {
        [
            ZoomLevel::Galaxy,
            ZoomLevel::Constellation,
            ZoomLevel::Overview,
            ZoomLevel::Standard,
            ZoomLevel::Micro,
        ]
    }
}

impl fmt::Display for ZoomLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ---------------------------------------------------------------------------
// ZoomConstraints
// ---------------------------------------------------------------------------

/// Hard limits and interaction rules applied at a given [`ZoomLevel`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomConstraints {
    /// Minimum allowed scale factor for the canvas.
    pub scale_min: f64,
    /// Maximum allowed scale factor for the canvas.
    pub scale_max: f64,
    /// Maximum scroll velocity (canvas units / ms) at this level.
    pub max_scroll_velocity: f64,
    /// Whether drag-to-pan is enabled.
    pub pan_enabled: bool,
    /// Whether node selection is permitted.
    pub selection_enabled: bool,
    /// Whether lasso selection is permitted.
    pub lasso_enabled: bool,
    /// Whether edge editing is permitted.
    pub edge_edit_enabled: bool,
    /// Maximum transition duration in milliseconds (≤16 ms → ≥60 FPS).
    pub transition_max_ms: u32,
}

impl ZoomConstraints {
    /// Default constraints enforced at the given zoom level.
    pub fn for_level(level: ZoomLevel) -> Self {
        let (scale_min, scale_max) = level.scale_range();
        match level {
            ZoomLevel::Galaxy => Self {
                scale_min,
                scale_max,
                max_scroll_velocity: 500.0,
                pan_enabled: true,
                selection_enabled: false,
                lasso_enabled: false,
                edge_edit_enabled: false,
                transition_max_ms: 16,
            },
            ZoomLevel::Constellation => Self {
                scale_min,
                scale_max,
                max_scroll_velocity: 300.0,
                pan_enabled: true,
                selection_enabled: false,
                lasso_enabled: false,
                edge_edit_enabled: false,
                transition_max_ms: 16,
            },
            ZoomLevel::Overview => Self {
                scale_min,
                scale_max,
                max_scroll_velocity: 150.0,
                pan_enabled: true,
                selection_enabled: true,
                lasso_enabled: true,
                edge_edit_enabled: false,
                transition_max_ms: 16,
            },
            ZoomLevel::Standard => Self {
                scale_min,
                scale_max,
                max_scroll_velocity: 80.0,
                pan_enabled: true,
                selection_enabled: true,
                lasso_enabled: true,
                edge_edit_enabled: true,
                transition_max_ms: 16,
            },
            ZoomLevel::Micro => Self {
                scale_min,
                scale_max,
                max_scroll_velocity: 40.0,
                pan_enabled: true,
                selection_enabled: true,
                lasso_enabled: true,
                edge_edit_enabled: true,
                transition_max_ms: 16,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// ZoomState
// ---------------------------------------------------------------------------

/// Runtime mutable zoom state for a single canvas viewport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomState {
    /// Current absolute scale factor.
    scale: f64,
    /// Current zoom level derived from `scale`.
    level: ZoomLevel,
    /// Origin in canvas space that the viewport is centred on (x, y).
    origin: (f64, f64),
}

impl ZoomState {
    /// Create a new [`ZoomState`] at `scale = 1.0` centred on the origin.
    pub fn new() -> Self {
        Self {
            scale: 1.0,
            level: ZoomLevel::Standard,
            origin: (0.0, 0.0),
        }
    }

    /// Current scale factor.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Current [`ZoomLevel`].
    pub fn level(&self) -> ZoomLevel {
        self.level
    }

    /// Canvas-space origin the viewport is currently focused on.
    pub fn origin(&self) -> (f64, f64) {
        self.origin
    }

    /// Apply a multiplicative zoom delta centred on `focal_point`.
    ///
    /// The resulting scale is clamped to the global supported range
    /// `[SCALE_MIN, SCALE_MAX)`.
    ///
    /// # Errors
    ///
    /// Returns [`ZoomError::DeltaIsZero`] when `delta == 0.0`.
    pub fn zoom(&mut self, delta: f64, focal_point: (f64, f64)) -> Result<(), ZoomError> {
        if delta == 0.0 {
            return Err(ZoomError::DeltaIsZero);
        }
        let new_scale = (self.scale * delta).clamp(SCALE_MIN, SCALE_MAX);
        // Adjust origin so the focal point stays fixed in canvas space.
        let ratio = new_scale / self.scale;
        self.origin = (
            focal_point.0 - (focal_point.0 - self.origin.0) * ratio,
            focal_point.1 - (focal_point.1 - self.origin.1) * ratio,
        );
        self.scale = new_scale;
        self.level = ZoomLevel::from_scale(new_scale)?;
        tracing::debug!(scale = self.scale, level = %self.level, "zoom applied");
        Ok(())
    }

    /// Pan the viewport by `(dx, dy)` canvas units.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.origin.0 += dx;
        self.origin.1 += dy;
    }

    /// Jump directly to a specific zoom level, recentring on `focal_point`.
    pub fn jump_to_level(&mut self, level: ZoomLevel, focal_point: (f64, f64)) {
        let (lo, hi) = level.scale_range();
        let mid = (lo + hi) / 2.0;
        self.scale = mid;
        self.level = level;
        self.origin = focal_point;
    }
}

impl Default for ZoomState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Absolute minimum scale factor across all zoom levels.
pub const SCALE_MIN: f64 = 0.01;
/// Absolute maximum scale factor across all zoom levels (exclusive upper bound).
pub const SCALE_MAX: f64 = 8.00;
/// Target frame budget in milliseconds (≥60 FPS).
pub const FRAME_BUDGET_MS: u32 = 16;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by zoom operations.
#[derive(Debug, Error, PartialEq)]
pub enum ZoomError {
    /// Scale factor is outside the supported range.
    #[error("scale {scale} is outside supported range [{}, {})", SCALE_MIN, SCALE_MAX)]
    ScaleOutOfBounds {
        /// The out-of-range scale value.
        scale: f64,
    },
    /// Zoom delta must be non-zero.
    #[error("zoom delta must be non-zero")]
    DeltaIsZero,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_scale_covers_all_levels() {
        let cases = [
            (0.01, ZoomLevel::Galaxy),
            (0.03, ZoomLevel::Galaxy),
            (0.10, ZoomLevel::Constellation),
            (0.40, ZoomLevel::Overview),
            (1.00, ZoomLevel::Standard),
            (4.00, ZoomLevel::Micro),
        ];
        for (scale, expected) in cases {
            assert_eq!(ZoomLevel::from_scale(scale).unwrap(), expected);
        }
    }

    #[test]
    fn from_scale_out_of_range_errors() {
        assert!(ZoomLevel::from_scale(0.001).is_err());
        assert!(ZoomLevel::from_scale(10.0).is_err());
    }

    #[test]
    fn zoom_state_zoom_and_level_update() {
        let mut state = ZoomState::new();
        assert_eq!(state.level(), ZoomLevel::Standard);
        // Zoom in past the Standard→Micro boundary.
        state.zoom(3.0, (0.0, 0.0)).unwrap();
        assert_eq!(state.level(), ZoomLevel::Micro);
    }

    #[test]
    fn zoom_state_clamped_within_bounds() {
        let mut state = ZoomState::new();
        // Zoom out massively.
        state.zoom(0.001, (0.0, 0.0)).unwrap();
        assert!(state.scale() >= SCALE_MIN);
        // Zoom in massively.
        state.zoom(1000.0, (0.0, 0.0)).unwrap();
        assert!(state.scale() <= SCALE_MAX);
    }

    #[test]
    fn zoom_delta_zero_errors() {
        let mut state = ZoomState::new();
        assert_eq!(state.zoom(0.0, (0.0, 0.0)), Err(ZoomError::DeltaIsZero));
    }

    #[test]
    fn pan_updates_origin() {
        let mut state = ZoomState::new();
        state.pan(10.0, -5.0);
        assert_eq!(state.origin(), (10.0, -5.0));
    }

    #[test]
    fn constraints_selection_disabled_at_galaxy() {
        let c = ZoomConstraints::for_level(ZoomLevel::Galaxy);
        assert!(!c.selection_enabled);
        assert!(!c.lasso_enabled);
    }

    #[test]
    fn constraints_all_enabled_at_standard() {
        let c = ZoomConstraints::for_level(ZoomLevel::Standard);
        assert!(c.selection_enabled);
        assert!(c.lasso_enabled);
        assert!(c.edge_edit_enabled);
    }

    #[test]
    fn transition_within_frame_budget() {
        for level in ZoomLevel::all() {
            let c = ZoomConstraints::for_level(level);
            assert!(
                c.transition_max_ms <= FRAME_BUDGET_MS,
                "transition budget for {level} exceeds {FRAME_BUDGET_MS} ms"
            );
        }
    }

    #[test]
    fn jump_to_level_sets_correct_level() {
        let mut state = ZoomState::new();
        state.jump_to_level(ZoomLevel::Galaxy, (100.0, 200.0));
        assert_eq!(state.level(), ZoomLevel::Galaxy);
        assert_eq!(state.origin(), (100.0, 200.0));
    }
}
