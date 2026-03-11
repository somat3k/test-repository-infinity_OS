//! # visibility — Node Visibility and Detail Scaling Policy
//!
//! Governs which nodes and UI elements are rendered at a given [`ZoomLevel`]
//! and how much detail they expose.  This implements the visibility culling
//! rules and level-of-detail (LoD) scaling policy for the infinity canvas.
//!
//! ## Policy summary
//!
//! | Zoom level    | Nodes shown          | Labels | Ports | Status overlay |
//! |---------------|----------------------|--------|-------|----------------|
//! | Galaxy        | Dimension clusters   | none   | no    | no             |
//! | Constellation | Group outlines       | group  | no    | no             |
//! | Overview      | Node chips (icon)    | short  | no    | icon           |
//! | Standard      | Full node cards      | full   | yes   | full           |
//! | Micro         | Node internals       | full   | yes   | full+debug     |

use serde::{Deserialize, Serialize};

use crate::zoom::ZoomLevel;

// ---------------------------------------------------------------------------
// DetailLevel
// ---------------------------------------------------------------------------

/// How much detail a node exposes at the current zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DetailLevel {
    /// Not rendered — outside the viewport or culled.
    Hidden,
    /// Dimension cluster badge only.
    ClusterBadge,
    /// Group bounding box + label.
    GroupOutline,
    /// Small node chip with icon and short label.
    Chip,
    /// Full node card with header, body, and connection ports.
    Card,
    /// Full card plus internal state, port types, and debug overlays.
    CardDebug,
}

impl DetailLevel {
    /// Derive the appropriate detail level for a node at the given zoom.
    pub fn for_zoom(zoom: ZoomLevel) -> Self {
        match zoom {
            ZoomLevel::Galaxy => DetailLevel::ClusterBadge,
            ZoomLevel::Constellation => DetailLevel::GroupOutline,
            ZoomLevel::Overview => DetailLevel::Chip,
            ZoomLevel::Standard => DetailLevel::Card,
            ZoomLevel::Micro => DetailLevel::CardDebug,
        }
    }

    /// Returns `true` if the node is visible (i.e., not [`DetailLevel::Hidden`]).
    pub fn is_visible(self) -> bool {
        self != DetailLevel::Hidden
    }

    /// Returns `true` if port labels should be rendered.
    pub fn show_ports(self) -> bool {
        matches!(self, DetailLevel::Card | DetailLevel::CardDebug)
    }

    /// Returns `true` if the full text label should be rendered.
    pub fn show_full_label(self) -> bool {
        matches!(self, DetailLevel::Card | DetailLevel::CardDebug)
    }

    /// Returns `true` if a status/health overlay should be rendered.
    pub fn show_status_overlay(self) -> bool {
        matches!(
            self,
            DetailLevel::Chip | DetailLevel::Card | DetailLevel::CardDebug
        )
    }

    /// Returns `true` if debug data (types, memory, timing) should be shown.
    pub fn show_debug_overlay(self) -> bool {
        self == DetailLevel::CardDebug
    }
}

// ---------------------------------------------------------------------------
// VisibilityPolicy
// ---------------------------------------------------------------------------

/// Determines whether a node or group should be rendered given the current
/// viewport and zoom state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibilityPolicy {
    /// Current zoom level.
    pub zoom_level: ZoomLevel,
    /// Viewport bounding box in canvas coordinates `(x_min, y_min, x_max, y_max)`.
    pub viewport: (f64, f64, f64, f64),
}

impl VisibilityPolicy {
    /// Create a new policy for the given zoom and viewport.
    pub fn new(zoom_level: ZoomLevel, viewport: (f64, f64, f64, f64)) -> Self {
        Self {
            zoom_level,
            viewport,
        }
    }

    /// Compute the [`DetailLevel`] for a node at a given position.
    ///
    /// A node is [`DetailLevel::Hidden`] when its bounding box does not
    /// intersect the viewport.
    pub fn detail_for_node(&self, node_rect: (f64, f64, f64, f64)) -> DetailLevel {
        if !self.intersects_viewport(node_rect) {
            return DetailLevel::Hidden;
        }
        DetailLevel::for_zoom(self.zoom_level)
    }

    /// Returns `true` if the given rectangle intersects the viewport.
    pub fn intersects_viewport(&self, rect: (f64, f64, f64, f64)) -> bool {
        let (vx0, vy0, vx1, vy1) = self.viewport;
        let (rx0, ry0, rx1, ry1) = rect;
        rx1 >= vx0 && rx0 <= vx1 && ry1 >= vy0 && ry0 <= vy1
    }

    /// Batch-evaluate detail levels for a list of node rectangles.
    ///
    /// Returns a `Vec<DetailLevel>` aligned with the input slice.
    pub fn batch_detail<'a>(
        &self,
        node_rects: impl IntoIterator<Item = &'a (f64, f64, f64, f64)>,
    ) -> Vec<DetailLevel> {
        node_rects
            .into_iter()
            .map(|&r| self.detail_for_node(r))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_level_for_each_zoom() {
        assert_eq!(DetailLevel::for_zoom(ZoomLevel::Galaxy), DetailLevel::ClusterBadge);
        assert_eq!(DetailLevel::for_zoom(ZoomLevel::Constellation), DetailLevel::GroupOutline);
        assert_eq!(DetailLevel::for_zoom(ZoomLevel::Overview), DetailLevel::Chip);
        assert_eq!(DetailLevel::for_zoom(ZoomLevel::Standard), DetailLevel::Card);
        assert_eq!(DetailLevel::for_zoom(ZoomLevel::Micro), DetailLevel::CardDebug);
    }

    #[test]
    fn ports_only_visible_at_card_levels() {
        assert!(!DetailLevel::ClusterBadge.show_ports());
        assert!(!DetailLevel::Chip.show_ports());
        assert!(DetailLevel::Card.show_ports());
        assert!(DetailLevel::CardDebug.show_ports());
    }

    #[test]
    fn debug_overlay_only_at_micro() {
        for level in [
            DetailLevel::Hidden,
            DetailLevel::ClusterBadge,
            DetailLevel::GroupOutline,
            DetailLevel::Chip,
            DetailLevel::Card,
        ] {
            assert!(!level.show_debug_overlay());
        }
        assert!(DetailLevel::CardDebug.show_debug_overlay());
    }

    #[test]
    fn viewport_culling() {
        let policy = VisibilityPolicy::new(
            ZoomLevel::Standard,
            (0.0, 0.0, 100.0, 100.0),
        );
        // Fully inside viewport.
        let visible = policy.detail_for_node((10.0, 10.0, 40.0, 40.0));
        assert!(visible.is_visible());
        // Completely outside viewport.
        let hidden = policy.detail_for_node((200.0, 200.0, 300.0, 300.0));
        assert_eq!(hidden, DetailLevel::Hidden);
        // Partially overlapping.
        let partial = policy.detail_for_node((80.0, 80.0, 120.0, 120.0));
        assert!(partial.is_visible());
    }

    #[test]
    fn batch_detail_aligns_with_individual() {
        let policy = VisibilityPolicy::new(ZoomLevel::Overview, (0.0, 0.0, 50.0, 50.0));
        let rects = vec![
            (5.0, 5.0, 20.0, 20.0),   // inside
            (60.0, 60.0, 80.0, 80.0), // outside
        ];
        let batch = policy.batch_detail(rects.iter());
        assert!(batch[0].is_visible());
        assert_eq!(batch[1], DetailLevel::Hidden);
    }
}
