//! # navigation — Minimap, Breadcrumbs, and Focus Mode
//!
//! Provides the data model and state for the three navigation affordances:
//!
//! - **Minimap** — a thumbnail overview of the full canvas with a viewport indicator.
//! - **Breadcrumbs** — a trail of dimension → group → node scopes currently in focus.
//! - **Focus Mode** — temporarily hides everything outside the focused subgraph.

use serde::{Deserialize, Serialize};

use crate::selection::Rect;

// ---------------------------------------------------------------------------
// Minimap
// ---------------------------------------------------------------------------

/// Minimap state: controls visibility and the current viewport rect.
///
/// The minimap renderer uses `canvas_bounds` and `viewport_rect` to compute
/// the proportional indicator box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Minimap {
    /// Whether the minimap widget is visible.
    pub visible: bool,
    /// Bounding box of the full canvas content, in canvas coordinates.
    pub canvas_bounds: Rect,
    /// Current viewport rect in canvas coordinates.
    pub viewport_rect: Rect,
}

impl Minimap {
    /// Create a minimap with the given canvas bounds, hidden by default.
    pub fn new(canvas_bounds: Rect) -> Self {
        Self {
            visible: false,
            canvas_bounds,
            viewport_rect: canvas_bounds,
        }
    }

    /// Toggle minimap visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Update the viewport rect (called on every pan/zoom change).
    pub fn update_viewport(&mut self, viewport: Rect) {
        self.viewport_rect = viewport;
    }

    /// Compute the minimap indicator rect in minimap widget coordinates.
    ///
    /// `minimap_size` is the pixel dimensions of the minimap widget
    /// `(widget_width, widget_height)`.
    ///
    /// Returns `(x, y, w, h)` in widget pixels.
    pub fn indicator_rect(&self, minimap_size: (f64, f64)) -> (f64, f64, f64, f64) {
        let cb = &self.canvas_bounds;
        let vp = &self.viewport_rect;
        let (mw, mh) = minimap_size;

        let scale_x = mw / cb.width.max(1.0);
        let scale_y = mh / cb.height.max(1.0);

        let x = (vp.origin.x - cb.origin.x) * scale_x;
        let y = (vp.origin.y - cb.origin.y) * scale_y;
        let w = vp.width * scale_x;
        let h = vp.height * scale_y;

        (x, y, w, h)
    }
}

// ---------------------------------------------------------------------------
// BreadcrumbEntry
// ---------------------------------------------------------------------------

/// A single entry in the canvas breadcrumb trail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreadcrumbEntry {
    /// Human-readable label for this scope level.
    pub label: String,
    /// The kind of scope (Dimension, Group, or Node).
    pub kind: BreadcrumbKind,
    /// Opaque identifier for the scope.
    pub id: String,
}

/// The kind of a breadcrumb scope level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreadcrumbKind {
    /// Top-level dimension.
    Dimension,
    /// A group within the dimension.
    Group,
    /// A specific node.
    Node,
}

/// The breadcrumb trail for the current canvas navigation context.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Breadcrumbs {
    entries: Vec<BreadcrumbEntry>,
}

impl Breadcrumbs {
    /// Create empty breadcrumbs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new scope onto the trail.
    pub fn push(&mut self, entry: BreadcrumbEntry) {
        self.entries.push(entry);
    }

    /// Pop the innermost scope.
    pub fn pop(&mut self) -> Option<BreadcrumbEntry> {
        self.entries.pop()
    }

    /// Navigate back to a specific depth (0-indexed from root).
    ///
    /// Removes all entries deeper than `depth`.
    pub fn navigate_to(&mut self, depth: usize) {
        self.entries.truncate(depth + 1);
    }

    /// Return the current trail.
    pub fn trail(&self) -> &[BreadcrumbEntry] {
        &self.entries
    }

    /// Depth of the trail (0 = empty, 1 = dimension only, …).
    pub fn depth(&self) -> usize {
        self.entries.len()
    }

    /// The innermost (leaf) entry.
    pub fn current(&self) -> Option<&BreadcrumbEntry> {
        self.entries.last()
    }
}

// ---------------------------------------------------------------------------
// FocusMode
// ---------------------------------------------------------------------------

/// Focus mode hides all canvas content outside the specified node ID set,
/// allowing the user to concentrate on a subgraph.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FocusMode {
    /// Whether focus mode is currently active.
    pub active: bool,
    /// The set of node IDs that remain fully visible in focus mode.
    pub focused_nodes: Vec<String>,
}

impl FocusMode {
    /// Create a focus mode struct (inactive by default).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter focus mode for the given node IDs.
    ///
    /// Any node not in `node_ids` will be dimmed or hidden by the renderer.
    pub fn enter(&mut self, node_ids: impl IntoIterator<Item = impl Into<String>>) {
        self.focused_nodes = node_ids.into_iter().map(Into::into).collect();
        self.active = true;
    }

    /// Exit focus mode, restoring full canvas visibility.
    pub fn exit(&mut self) {
        self.active = false;
        self.focused_nodes.clear();
    }

    /// Returns `true` if `node_id` should be fully rendered in the current mode.
    pub fn is_node_visible(&self, node_id: &str) -> bool {
        !self.active || self.focused_nodes.iter().any(|n| n == node_id)
    }
}

// ---------------------------------------------------------------------------
// NavigationState
// ---------------------------------------------------------------------------

/// Aggregated navigation state for the canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationState {
    /// Minimap state.
    pub minimap: Minimap,
    /// Breadcrumb trail.
    pub breadcrumbs: Breadcrumbs,
    /// Focus mode.
    pub focus_mode: FocusMode,
}

impl NavigationState {
    /// Create a default navigation state with the given canvas bounds.
    pub fn new(canvas_bounds: Rect) -> Self {
        Self {
            minimap: Minimap::new(canvas_bounds),
            breadcrumbs: Breadcrumbs::new(),
            focus_mode: FocusMode::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Rect;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn minimap_toggle() {
        let mut mm = Minimap::new(rect(0.0, 0.0, 1000.0, 800.0));
        assert!(!mm.visible);
        mm.toggle();
        assert!(mm.visible);
        mm.toggle();
        assert!(!mm.visible);
    }

    #[test]
    fn minimap_indicator_proportional() {
        let canvas = rect(0.0, 0.0, 1000.0, 800.0);
        let mut mm = Minimap::new(canvas);
        mm.update_viewport(rect(250.0, 200.0, 500.0, 400.0));
        let (x, y, w, h) = mm.indicator_rect((100.0, 80.0));
        // 250/1000 * 100 = 25
        assert!((x - 25.0).abs() < 1e-9);
        assert!((y - 20.0).abs() < 1e-9);
        assert!((w - 50.0).abs() < 1e-9);
        assert!((h - 40.0).abs() < 1e-9);
    }

    #[test]
    fn breadcrumbs_push_pop_navigate() {
        let mut bc = Breadcrumbs::new();
        bc.push(BreadcrumbEntry {
            label: "dim-1".into(),
            kind: BreadcrumbKind::Dimension,
            id: "d1".into(),
        });
        bc.push(BreadcrumbEntry {
            label: "group-a".into(),
            kind: BreadcrumbKind::Group,
            id: "g1".into(),
        });
        bc.push(BreadcrumbEntry {
            label: "node-x".into(),
            kind: BreadcrumbKind::Node,
            id: "n1".into(),
        });
        assert_eq!(bc.depth(), 3);
        bc.navigate_to(1); // back to group level
        assert_eq!(bc.depth(), 2);
        assert_eq!(bc.current().unwrap().id, "g1");
    }

    #[test]
    fn focus_mode_enter_exit() {
        let mut fm = FocusMode::new();
        fm.enter(["a", "b"]);
        assert!(fm.active);
        assert!(fm.is_node_visible("a"));
        assert!(!fm.is_node_visible("c"));
        fm.exit();
        assert!(!fm.active);
        assert!(fm.is_node_visible("c")); // everything visible when inactive
    }
}
