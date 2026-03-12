//! # datasets — Deterministic Test Datasets
//!
//! Provides stable, seeded graph and data-pipeline fixtures used across
//! unit, integration, and golden tests.  Every factory function is
//! **deterministic**: the same call always returns the same dataset, enabling
//! reproducible regression detection.
//!
//! All data is self-contained (no network, no real filesystem required).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Graph fixtures
// ---------------------------------------------------------------------------

/// A lightweight node in a test graph fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureNode {
    /// Unique node ID within the graph.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Arbitrary metadata key-value pairs.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl FixtureNode {
    fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            metadata: HashMap::new(),
        }
    }

    fn with_meta(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

/// A directed edge in a test graph fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureEdge {
    /// Source node ID.
    pub from: String,
    /// Target node ID.
    pub to: String,
    /// Optional edge label.
    pub label: Option<String>,
}

impl FixtureEdge {
    fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self { from: from.into(), to: to.into(), label: None }
    }

    fn labeled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// A deterministic graph fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphFixture {
    /// Dataset identifier.
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Nodes in the graph.
    pub nodes: Vec<FixtureNode>,
    /// Directed edges in the graph.
    pub edges: Vec<FixtureEdge>,
}

impl GraphFixture {
    /// **Fixture `"linear-3"`**: a straight-line graph of 3 nodes.
    ///
    /// ```text
    ///  A -> B -> C
    /// ```
    pub fn linear_three() -> Self {
        Self {
            id: "linear-3".into(),
            description: "Straight-line graph: A → B → C".into(),
            nodes: vec![
                FixtureNode::new("A", "Start").with_meta("kind", serde_json::json!("source")),
                FixtureNode::new("B", "Transform").with_meta("kind", serde_json::json!("process")),
                FixtureNode::new("C", "Sink").with_meta("kind", serde_json::json!("sink")),
            ],
            edges: vec![
                FixtureEdge::new("A", "B").labeled("flow"),
                FixtureEdge::new("B", "C").labeled("flow"),
            ],
        }
    }

    /// **Fixture `"diamond"`**: a diamond-shaped graph with fan-out and fan-in.
    ///
    /// ```text
    ///     A
    ///    / \
    ///   B   C
    ///    \ /
    ///     D
    /// ```
    pub fn diamond() -> Self {
        Self {
            id: "diamond".into(),
            description: "Diamond graph: A → B, A → C, B → D, C → D".into(),
            nodes: vec![
                FixtureNode::new("A", "Root"),
                FixtureNode::new("B", "Left"),
                FixtureNode::new("C", "Right"),
                FixtureNode::new("D", "Join"),
            ],
            edges: vec![
                FixtureEdge::new("A", "B").labeled("left"),
                FixtureEdge::new("A", "C").labeled("right"),
                FixtureEdge::new("B", "D").labeled("merge"),
                FixtureEdge::new("C", "D").labeled("merge"),
            ],
        }
    }

    /// **Fixture `"cycle"`**: a graph containing a back-edge (useful for cycle-detection tests).
    ///
    /// ```text
    ///  X -> Y -> Z -> X (cycle)
    /// ```
    pub fn cycle() -> Self {
        Self {
            id: "cycle".into(),
            description: "Cyclic graph: X → Y → Z → X".into(),
            nodes: vec![
                FixtureNode::new("X", "X"),
                FixtureNode::new("Y", "Y"),
                FixtureNode::new("Z", "Z"),
            ],
            edges: vec![
                FixtureEdge::new("X", "Y"),
                FixtureEdge::new("Y", "Z"),
                FixtureEdge::new("Z", "X"),
            ],
        }
    }

    /// **Fixture `"empty"`**: a graph with no nodes or edges (boundary case).
    pub fn empty() -> Self {
        Self {
            id: "empty".into(),
            description: "Empty graph: no nodes, no edges".into(),
            nodes: vec![],
            edges: vec![],
        }
    }

    /// **Fixture `"single-node"`**: a graph with exactly one isolated node.
    pub fn single_node() -> Self {
        Self {
            id: "single-node".into(),
            description: "Single isolated node".into(),
            nodes: vec![FixtureNode::new("N", "Lone")],
            edges: vec![],
        }
    }

    /// Return all built-in graph fixtures.
    pub fn all() -> Vec<Self> {
        vec![
            Self::linear_three(),
            Self::diamond(),
            Self::cycle(),
            Self::empty(),
            Self::single_node(),
        ]
    }

    /// Whether this graph contains a cycle (detected via DFS).
    pub fn has_cycle(&self) -> bool {
        // Build adjacency list.
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for e in &self.edges {
            adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
        }
        let mut visited: HashMap<&str, bool> = HashMap::new();
        let mut rec_stack: HashMap<&str, bool> = HashMap::new();

        fn dfs<'a>(
            node: &'a str,
            adj: &HashMap<&'a str, Vec<&'a str>>,
            visited: &mut HashMap<&'a str, bool>,
            rec_stack: &mut HashMap<&'a str, bool>,
        ) -> bool {
            visited.insert(node, true);
            rec_stack.insert(node, true);
            if let Some(neighbours) = adj.get(node) {
                for &nb in neighbours {
                    if !visited.get(nb).copied().unwrap_or(false) {
                        if dfs(nb, adj, visited, rec_stack) {
                            return true;
                        }
                    } else if rec_stack.get(nb).copied().unwrap_or(false) {
                        return true;
                    }
                }
            }
            rec_stack.insert(node, false);
            false
        }

        for node in &self.nodes {
            if !visited.get(node.id.as_str()).copied().unwrap_or(false)
                && dfs(&node.id, &adj, &mut visited, &mut rec_stack)
            {
                return true;
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Data-pipeline fixtures
// ---------------------------------------------------------------------------

/// A single record in a tabular dataset fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataRecord {
    /// Row index (0-based, stable across runs).
    pub index: usize,
    /// Field values keyed by column name.
    pub fields: HashMap<String, serde_json::Value>,
}

impl DataRecord {
    fn new(index: usize, fields: impl IntoIterator<Item = (impl Into<String>, serde_json::Value)>) -> Self {
        Self {
            index,
            fields: fields.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        }
    }
}

/// A deterministic tabular dataset fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetFixture {
    /// Dataset identifier.
    pub id: String,
    /// Column names in declared order.
    pub columns: Vec<String>,
    /// Rows.
    pub records: Vec<DataRecord>,
}

impl DatasetFixture {
    /// **Fixture `"timeseries-5"`**: 5 rows of (timestamp, value) time-series data.
    pub fn timeseries_five() -> Self {
        let columns = vec!["timestamp".into(), "value".into(), "label".into()];
        let records = (0..5)
            .map(|i| {
                DataRecord::new(
                    i,
                    [
                        ("timestamp", serde_json::json!(1_700_000_000u64 + i as u64 * 60)),
                        ("value", serde_json::json!(i as f64 * 1.5)),
                        ("label", serde_json::json!(format!("point-{i}"))),
                    ],
                )
            })
            .collect();
        Self { id: "timeseries-5".into(), columns, records }
    }

    /// **Fixture `"kv-pairs-10"`**: 10 rows of (key, value) pairs.
    pub fn kv_pairs_ten() -> Self {
        let columns = vec!["key".into(), "value".into()];
        let records = (0..10)
            .map(|i| {
                DataRecord::new(
                    i,
                    [
                        ("key", serde_json::json!(format!("k{i:02}"))),
                        ("value", serde_json::json!(i * i)),
                    ],
                )
            })
            .collect();
        Self { id: "kv-pairs-10".into(), columns, records }
    }

    /// **Fixture `"empty-dataset"`**: zero rows (boundary case).
    pub fn empty() -> Self {
        Self {
            id: "empty-dataset".into(),
            columns: vec!["id".into(), "payload".into()],
            records: vec![],
        }
    }

    /// Return all built-in dataset fixtures.
    pub fn all() -> Vec<Self> {
        vec![Self::timeseries_five(), Self::kv_pairs_ten(), Self::empty()]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_three_has_no_cycle() {
        assert!(!GraphFixture::linear_three().has_cycle());
    }

    #[test]
    fn diamond_has_no_cycle() {
        assert!(!GraphFixture::diamond().has_cycle());
    }

    #[test]
    fn cycle_is_detected() {
        assert!(GraphFixture::cycle().has_cycle());
    }

    #[test]
    fn empty_graph_has_no_cycle() {
        assert!(!GraphFixture::empty().has_cycle());
    }

    #[test]
    fn all_graph_fixtures_are_deterministic() {
        let first = GraphFixture::all();
        let second = GraphFixture::all();
        assert_eq!(first, second);
    }

    #[test]
    fn timeseries_fixture_has_five_rows() {
        let ds = DatasetFixture::timeseries_five();
        assert_eq!(ds.records.len(), 5);
    }

    #[test]
    fn kv_pairs_fixture_is_deterministic() {
        let a = DatasetFixture::kv_pairs_ten();
        let b = DatasetFixture::kv_pairs_ten();
        assert_eq!(a, b);
    }

    #[test]
    fn all_dataset_fixtures_round_trip_json() {
        for ds in DatasetFixture::all() {
            let json = serde_json::to_string(&ds).expect("serialize");
            let back: DatasetFixture = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(ds, back);
        }
    }
}
