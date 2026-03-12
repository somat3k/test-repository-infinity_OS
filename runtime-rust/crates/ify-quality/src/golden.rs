//! # golden — Golden Tests for UI Layouts
//!
//! Provides the snapshot capture, storage, and comparison infrastructure for
//! golden (snapshot) tests of infinityOS canvas UI layouts.
//!
//! A *golden file* is a checked-in reference snapshot for a named UI
//! component or canvas view.  During test runs the current render output is
//! compared against the golden; any diff causes a test failure.  Snapshots
//! are updated explicitly with [`GoldenStore::update`] (never automatically).
//!
//! Snapshots are text-based (JSON or SVG-like layout trees) to keep diffs
//! readable in pull-request reviews.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the golden testing module.
#[derive(Debug, Error)]
pub enum GoldenError {
    /// No golden snapshot exists for the given name.
    #[error("no golden snapshot found for '{0}'; run with UPDATE_GOLDEN=1 to create it")]
    SnapshotMissing(String),
    /// The actual output does not match the golden snapshot.
    #[error("golden mismatch for '{name}'\n--- expected ---\n{expected}\n--- actual ---\n{actual}")]
    SnapshotMismatch {
        /// Snapshot name.
        name: String,
        /// Stored (expected) value.
        expected: String,
        /// Current (actual) value.
        actual: String,
    },
}

// ---------------------------------------------------------------------------
// Layout node (minimal UI layout tree)
// ---------------------------------------------------------------------------

/// A node in a UI layout tree used for golden snapshotting.
///
/// Intentionally minimal: we capture only the properties that affect visual
/// layout (component type, bounds, key styling attributes, children).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutNode {
    /// Component identifier.
    pub component: String,
    /// Bounding box: `[x, y, width, height]` in canvas units.
    pub bounds: [f32; 4],
    /// Key attributes (class names, visibility, text content, etc.).
    pub attrs: HashMap<String, String>,
    /// Child layout nodes.
    pub children: Vec<LayoutNode>,
}

impl LayoutNode {
    /// Create a simple leaf layout node.
    pub fn leaf(component: impl Into<String>, bounds: [f32; 4]) -> Self {
        Self {
            component: component.into(),
            bounds,
            attrs: HashMap::new(),
            children: vec![],
        }
    }

    /// Attach an attribute.
    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attrs.insert(key.into(), value.into());
        self
    }

    /// Attach a child node.
    pub fn with_child(mut self, child: LayoutNode) -> Self {
        self.children.push(child);
        self
    }

    /// Serialize to a stable, human-readable JSON string for snapshotting.
    ///
    /// In the unlikely event that serialization fails (which cannot happen for
    /// well-formed `LayoutNode` values), a best-effort error JSON is returned
    /// instead of panicking.
    pub fn to_snapshot_string(&self) -> String {
        match serde_json::to_string_pretty(self) {
            Ok(json) => json,
            Err(err) => {
                format!(
                    "{{\"serialization_error\":\"{}\",\"component\":\"{}\"}}",
                    err, self.component
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Golden store
// ---------------------------------------------------------------------------

/// In-memory store for golden snapshots.
///
/// In a real test environment the store is backed by files on disk under
/// `tests/fixtures/golden/`.  This struct models the in-process state that
/// the test harness uses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoldenStore {
    snapshots: HashMap<String, String>,
}

impl GoldenStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Manually insert a golden snapshot (used to pre-populate the store in tests).
    pub fn insert(&mut self, name: impl Into<String>, snapshot: impl Into<String>) {
        self.snapshots.insert(name.into(), snapshot.into());
    }

    /// Update (or create) the golden snapshot for `name` with `actual`.
    ///
    /// This is the "bless" operation: it unconditionally writes the new value.
    pub fn update(&mut self, name: impl Into<String>, actual: impl Into<String>) {
        self.snapshots.insert(name.into(), actual.into());
    }

    /// Assert that `actual` matches the stored golden for `name`.
    ///
    /// # Errors
    /// - [`GoldenError::SnapshotMissing`]: no snapshot stored for `name`.
    /// - [`GoldenError::SnapshotMismatch`]: stored value ≠ `actual`.
    pub fn assert_matches(&self, name: &str, actual: &str) -> Result<(), GoldenError> {
        let expected = self
            .snapshots
            .get(name)
            .ok_or_else(|| GoldenError::SnapshotMissing(name.to_string()))?;

        if expected == actual {
            Ok(())
        } else {
            Err(GoldenError::SnapshotMismatch {
                name: name.to_string(),
                expected: expected.clone(),
                actual: actual.to_string(),
            })
        }
    }

    /// Return the stored snapshot for `name`, if any.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.snapshots.get(name).map(String::as_str)
    }
}

// ---------------------------------------------------------------------------
// Built-in layout fixtures
// ---------------------------------------------------------------------------

/// Factory functions for deterministic canvas layout fixtures used in golden tests.
pub struct CanvasLayoutFixtures;

impl CanvasLayoutFixtures {
    /// Minimal canvas viewport with a single leaf node at standard zoom.
    pub fn single_node_standard_zoom() -> LayoutNode {
        LayoutNode::leaf("canvas-viewport", [0.0, 0.0, 1280.0, 800.0])
            .with_attr("zoom", "1.0")
            .with_attr("theme", "dark")
            .with_child(
                LayoutNode::leaf("canvas-node", [100.0, 100.0, 160.0, 80.0])
                    .with_attr("label", "Start")
                    .with_attr("status", "idle"),
            )
    }

    /// Diamond graph layout with 4 nodes.
    pub fn diamond_graph_layout() -> LayoutNode {
        LayoutNode::leaf("canvas-viewport", [0.0, 0.0, 1280.0, 800.0])
            .with_attr("zoom", "1.0")
            .with_child(
                LayoutNode::leaf("canvas-node", [400.0, 50.0, 160.0, 80.0])
                    .with_attr("label", "Root")
                    .with_attr("id", "A"),
            )
            .with_child(
                LayoutNode::leaf("canvas-node", [200.0, 200.0, 160.0, 80.0])
                    .with_attr("label", "Left")
                    .with_attr("id", "B"),
            )
            .with_child(
                LayoutNode::leaf("canvas-node", [600.0, 200.0, 160.0, 80.0])
                    .with_attr("label", "Right")
                    .with_attr("id", "C"),
            )
            .with_child(
                LayoutNode::leaf("canvas-node", [400.0, 350.0, 160.0, 80.0])
                    .with_attr("label", "Join")
                    .with_attr("id", "D"),
            )
    }

    /// Inspector panel with node details.
    pub fn inspector_panel() -> LayoutNode {
        LayoutNode::leaf("inspector-panel", [900.0, 0.0, 380.0, 800.0])
            .with_attr("visible", "true")
            .with_child(
                LayoutNode::leaf("inspector-header", [900.0, 0.0, 380.0, 48.0])
                    .with_attr("title", "Node Inspector"),
            )
            .with_child(
                LayoutNode::leaf("inspector-body", [900.0, 48.0, 380.0, 752.0])
                    .with_attr("section", "parameters"),
            )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_match_passes() {
        let mut store = GoldenStore::new();
        let snapshot = "hello golden";
        store.insert("test-view", snapshot);
        assert!(store.assert_matches("test-view", snapshot).is_ok());
    }

    #[test]
    fn golden_mismatch_errors() {
        let mut store = GoldenStore::new();
        store.insert("test-view", "expected output");
        let err = store.assert_matches("test-view", "different output");
        assert!(matches!(err, Err(GoldenError::SnapshotMismatch { .. })));
    }

    #[test]
    fn golden_missing_errors() {
        let store = GoldenStore::new();
        let err = store.assert_matches("nonexistent", "anything");
        assert!(matches!(err, Err(GoldenError::SnapshotMissing(_))));
    }

    #[test]
    fn update_replaces_existing_snapshot() {
        let mut store = GoldenStore::new();
        store.insert("v", "old");
        store.update("v", "new");
        assert_eq!(store.get("v"), Some("new"));
    }

    #[test]
    fn layout_fixtures_are_deterministic() {
        let a = CanvasLayoutFixtures::single_node_standard_zoom();
        let b = CanvasLayoutFixtures::single_node_standard_zoom();
        assert_eq!(a, b);
    }

    #[test]
    fn layout_node_to_snapshot_string_is_stable() {
        let node = CanvasLayoutFixtures::inspector_panel();
        let s1 = node.to_snapshot_string();
        let s2 = node.to_snapshot_string();
        assert_eq!(s1, s2);
        assert!(!s1.is_empty());
    }

    #[test]
    fn golden_store_roundtrip_with_layout_fixture() {
        let mut store = GoldenStore::new();
        let node = CanvasLayoutFixtures::diamond_graph_layout();
        let snapshot = node.to_snapshot_string();
        store.update("diamond-layout", &snapshot);
        assert!(store.assert_matches("diamond-layout", &snapshot).is_ok());
    }
}
