//! # selection — Multi-Select, Lasso, Snap-to-Grid, Align/Distribute
//!
//! Implements the canvas selection toolkit: individual node selection,
//! rectangular lasso selection, snap-to-grid positioning, and
//! align/distribute operations on selected nodes.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Rect / Point
// ---------------------------------------------------------------------------

/// A 2-D point in canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal position.
    pub x: f64,
    /// Vertical position.
    pub y: f64,
}

impl Point {
    /// Create a new point.
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    /// Top-left corner.
    pub origin: Point,
    /// Width (must be ≥ 0).
    pub width: f64,
    /// Height (must be ≥ 0).
    pub height: f64,
}

impl Rect {
    /// Create a new rect.
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point::new(x, y),
            width,
            height,
        }
    }

    /// Returns `true` if `point` lies within (or on the boundary of) this rect.
    pub fn contains_point(&self, p: Point) -> bool {
        p.x >= self.origin.x
            && p.x <= self.origin.x + self.width
            && p.y >= self.origin.y
            && p.y <= self.origin.y + self.height
    }

    /// Returns `true` if `other` overlaps this rect.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.origin.x < other.origin.x + other.width
            && self.origin.x + self.width > other.origin.x
            && self.origin.y < other.origin.y + other.height
            && self.origin.y + self.height > other.origin.y
    }

    /// Centred x coordinate.
    pub fn center_x(&self) -> f64 {
        self.origin.x + self.width / 2.0
    }

    /// Centred y coordinate.
    pub fn center_y(&self) -> f64 {
        self.origin.y + self.height / 2.0
    }
}

// ---------------------------------------------------------------------------
// SelectionSet
// ---------------------------------------------------------------------------

/// The current set of selected node IDs.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SelectionSet {
    ids: HashSet<String>,
}

impl SelectionSet {
    /// Create an empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node to the selection.
    pub fn add(&mut self, id: impl Into<String>) {
        self.ids.insert(id.into());
    }

    /// Remove a node from the selection.
    pub fn remove(&mut self, id: &str) {
        self.ids.remove(id);
    }

    /// Toggle a node's selection state.
    pub fn toggle(&mut self, id: impl Into<String>) {
        let s: String = id.into();
        if self.ids.contains(&s) {
            self.ids.remove(&s);
        } else {
            self.ids.insert(s);
        }
    }

    /// Clear all selections.
    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Returns `true` if the node is selected.
    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Number of selected nodes.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Returns `true` if the selection is empty.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Iterate over selected node IDs.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.ids.iter().map(String::as_str)
    }
}

// ---------------------------------------------------------------------------
// LassoSelector
// ---------------------------------------------------------------------------

/// Performs rectangular lasso selection against a collection of node rects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LassoSelector {
    /// The lasso rectangle in canvas coordinates.
    pub lasso: Rect,
}

impl LassoSelector {
    /// Create a lasso selector with the given rectangle.
    pub fn new(lasso: Rect) -> Self {
        Self { lasso }
    }

    /// Return the IDs of all nodes whose bounding rect intersects the lasso.
    pub fn select<'a>(
        &self,
        nodes: impl IntoIterator<Item = (&'a str, &'a Rect)>,
    ) -> SelectionSet {
        let mut sel = SelectionSet::new();
        for (id, rect) in nodes {
            if self.lasso.intersects(rect) {
                sel.add(id);
            }
        }
        sel
    }
}

// ---------------------------------------------------------------------------
// SnapGrid
// ---------------------------------------------------------------------------

/// Snaps canvas positions to a configurable grid.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapGrid {
    /// Grid cell size in canvas units (must be > 0).
    pub cell_size: f64,
    /// Whether snapping is currently enabled.
    pub enabled: bool,
}

impl SnapGrid {
    /// Create a snap grid with the given cell size, enabled by default.
    pub fn new(cell_size: f64) -> Self {
        Self {
            cell_size,
            enabled: true,
        }
    }

    /// Snap `value` to the nearest grid line.
    pub fn snap(&self, value: f64) -> f64 {
        if !self.enabled || self.cell_size <= 0.0 {
            return value;
        }
        (value / self.cell_size).round() * self.cell_size
    }

    /// Snap a [`Point`] to the grid.
    pub fn snap_point(&self, p: Point) -> Point {
        Point::new(self.snap(p.x), self.snap(p.y))
    }
}

impl Default for SnapGrid {
    fn default() -> Self {
        Self::new(16.0)
    }
}

// ---------------------------------------------------------------------------
// AlignDistribute
// ---------------------------------------------------------------------------

/// Alignment axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    /// Horizontal axis.
    Horizontal,
    /// Vertical axis.
    Vertical,
}

/// Alignment anchor for a set of nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlignAnchor {
    /// Align to the minimum coordinate (left / top).
    Min,
    /// Align to the centre coordinate.
    Center,
    /// Align to the maximum coordinate (right / bottom).
    Max,
}

/// Errors from align/distribute operations.
#[derive(Debug, Error, PartialEq)]
pub enum AlignError {
    /// At least two nodes are required to align or distribute.
    #[error("at least two nodes are required")]
    TooFewNodes,
}

/// Applies alignment and distribution operations to a mutable list of node
/// positions and sizes.
pub struct AlignDistribute;

impl AlignDistribute {
    /// Align all nodes along `axis` to `anchor`.
    ///
    /// Modifies the positions in-place.
    ///
    /// # Errors
    ///
    /// Returns [`AlignError::TooFewNodes`] if fewer than 2 nodes are provided.
    pub fn align(
        nodes: &mut [Rect],
        axis: Axis,
        anchor: AlignAnchor,
    ) -> Result<(), AlignError> {
        if nodes.len() < 2 {
            return Err(AlignError::TooFewNodes);
        }
        let target = match (axis, anchor) {
            (Axis::Horizontal, AlignAnchor::Min) => {
                nodes.iter().map(|r| r.origin.x).fold(f64::INFINITY, f64::min)
            }
            (Axis::Horizontal, AlignAnchor::Center) => {
                nodes.iter().map(|r| r.center_x()).sum::<f64>() / nodes.len() as f64
            }
            (Axis::Horizontal, AlignAnchor::Max) => {
                nodes.iter().map(|r| r.origin.x + r.width).fold(f64::NEG_INFINITY, f64::max)
            }
            (Axis::Vertical, AlignAnchor::Min) => {
                nodes.iter().map(|r| r.origin.y).fold(f64::INFINITY, f64::min)
            }
            (Axis::Vertical, AlignAnchor::Center) => {
                nodes.iter().map(|r| r.center_y()).sum::<f64>() / nodes.len() as f64
            }
            (Axis::Vertical, AlignAnchor::Max) => {
                nodes.iter().map(|r| r.origin.y + r.height).fold(f64::NEG_INFINITY, f64::max)
            }
        };
        for node in nodes.iter_mut() {
            match (axis, anchor) {
                (Axis::Horizontal, AlignAnchor::Min) => node.origin.x = target,
                (Axis::Horizontal, AlignAnchor::Center) => {
                    node.origin.x = target - node.width / 2.0
                }
                (Axis::Horizontal, AlignAnchor::Max) => node.origin.x = target - node.width,
                (Axis::Vertical, AlignAnchor::Min) => node.origin.y = target,
                (Axis::Vertical, AlignAnchor::Center) => {
                    node.origin.y = target - node.height / 2.0
                }
                (Axis::Vertical, AlignAnchor::Max) => node.origin.y = target - node.height,
            }
        }
        Ok(())
    }

    /// Distribute nodes evenly along `axis`.
    ///
    /// # Errors
    ///
    /// Returns [`AlignError::TooFewNodes`] if fewer than 2 nodes are provided.
    pub fn distribute(nodes: &mut [Rect], axis: Axis) -> Result<(), AlignError> {
        if nodes.len() < 2 {
            return Err(AlignError::TooFewNodes);
        }
        match axis {
            Axis::Horizontal => {
                nodes.sort_by(|a, b| a.origin.x.partial_cmp(&b.origin.x).unwrap());
                let first_x = nodes.first().unwrap().origin.x;
                let last_x = nodes.last().unwrap().origin.x + nodes.last().unwrap().width;
                let total_w: f64 = nodes.iter().map(|r| r.width).sum();
                let gap = (last_x - first_x - total_w) / (nodes.len() as f64 - 1.0);
                let mut cursor = first_x;
                for node in nodes.iter_mut() {
                    node.origin.x = cursor;
                    cursor += node.width + gap;
                }
            }
            Axis::Vertical => {
                nodes.sort_by(|a, b| a.origin.y.partial_cmp(&b.origin.y).unwrap());
                let first_y = nodes.first().unwrap().origin.y;
                let last_y = nodes.last().unwrap().origin.y + nodes.last().unwrap().height;
                let total_h: f64 = nodes.iter().map(|r| r.height).sum();
                let gap = (last_y - first_y - total_h) / (nodes.len() as f64 - 1.0);
                let mut cursor = first_y;
                for node in nodes.iter_mut() {
                    node.origin.y = cursor;
                    cursor += node.height + gap;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_add_remove_toggle() {
        let mut sel = SelectionSet::new();
        sel.add("a");
        sel.add("b");
        assert!(sel.contains("a"));
        sel.remove("a");
        assert!(!sel.contains("a"));
        sel.toggle("b"); // remove
        assert!(!sel.contains("b"));
        sel.toggle("c"); // add
        assert!(sel.contains("c"));
    }

    #[test]
    fn lasso_selects_intersecting_nodes() {
        let lasso = LassoSelector::new(Rect::new(0.0, 0.0, 50.0, 50.0));
        let nodes = vec![
            ("inside", Rect::new(10.0, 10.0, 20.0, 20.0)),
            ("outside", Rect::new(100.0, 100.0, 20.0, 20.0)),
            ("partial", Rect::new(40.0, 40.0, 30.0, 30.0)),
        ];
        let sel = lasso.select(nodes.iter().map(|(id, r)| (*id, r)));
        assert!(sel.contains("inside"));
        assert!(!sel.contains("outside"));
        assert!(sel.contains("partial"));
    }

    #[test]
    fn snap_grid_rounds_to_cell() {
        let grid = SnapGrid::new(10.0);
        assert_eq!(grid.snap(13.0), 10.0);
        assert_eq!(grid.snap(15.0), 20.0);
        assert_eq!(grid.snap(0.0), 0.0);
    }

    #[test]
    fn snap_grid_disabled_passes_through() {
        let grid = SnapGrid {
            cell_size: 10.0,
            enabled: false,
        };
        assert_eq!(grid.snap(13.7), 13.7);
    }

    #[test]
    fn align_horizontal_min() {
        let mut nodes = vec![
            Rect::new(30.0, 0.0, 10.0, 10.0),
            Rect::new(10.0, 0.0, 10.0, 10.0),
            Rect::new(50.0, 0.0, 10.0, 10.0),
        ];
        AlignDistribute::align(&mut nodes, Axis::Horizontal, AlignAnchor::Min).unwrap();
        for n in &nodes {
            assert_eq!(n.origin.x, 10.0);
        }
    }

    #[test]
    fn distribute_horizontal_evenly() {
        let mut nodes = vec![
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(50.0, 0.0, 10.0, 10.0),
            Rect::new(100.0, 0.0, 10.0, 10.0),
        ];
        AlignDistribute::distribute(&mut nodes, Axis::Horizontal).unwrap();
        // After distribution the gaps should be equal.
        let gap1 = nodes[1].origin.x - (nodes[0].origin.x + nodes[0].width);
        let gap2 = nodes[2].origin.x - (nodes[1].origin.x + nodes[1].width);
        assert!((gap1 - gap2).abs() < 1e-9);
    }

    #[test]
    fn align_too_few_nodes_errors() {
        let mut nodes = vec![Rect::new(0.0, 0.0, 10.0, 10.0)];
        assert_eq!(
            AlignDistribute::align(&mut nodes, Axis::Vertical, AlignAnchor::Min),
            Err(AlignError::TooFewNodes)
        );
    }
}
