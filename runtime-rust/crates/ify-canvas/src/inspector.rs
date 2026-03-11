//! # inspector — Node Inspector Panel
//!
//! Provides the data model for the node inspector panel, which surfaces a
//! node's parameters, attached tools, memory usage, execution logs, and
//! mesh artifacts to the user.
//!
//! The inspector is populated by subscribing to the mesh artifact bus and
//! ActionLog stream for the inspected node.  All data is read-only from the
//! panel's perspective; mutations go through the node customizer.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Parameter
// ---------------------------------------------------------------------------

/// A single node parameter value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ParameterValue {
    /// UTF-8 string.
    String(String),
    /// 64-bit float.
    Float(f64),
    /// 64-bit integer.
    Int(i64),
    /// Boolean flag.
    Bool(bool),
    /// Arbitrary JSON value.
    Json(serde_json::Value),
}

/// A named parameter entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    /// Parameter name.
    pub name: String,
    /// Current value.
    pub value: ParameterValue,
    /// Optional description shown in the inspector.
    pub description: Option<String>,
}

impl Parameter {
    /// Create a new parameter.
    pub fn new(name: impl Into<String>, value: ParameterValue) -> Self {
        Self {
            name: name.into(),
            value,
            description: None,
        }
    }

    /// Attach a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

// ---------------------------------------------------------------------------
// ToolAttachment
// ---------------------------------------------------------------------------

/// A tool attached to the node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolAttachment {
    /// Tool identifier (e.g. `"db"`, `"http"`, `"model"`).
    pub tool_id: String,
    /// Human-readable tool name.
    pub display_name: String,
    /// Whether the tool is currently active.
    pub active: bool,
    /// Tool-specific configuration summary.
    pub config_summary: Option<String>,
}

// ---------------------------------------------------------------------------
// MemorySnapshot
// ---------------------------------------------------------------------------

/// Snapshot of the node's memory usage.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MemorySnapshot {
    /// Heap bytes currently allocated by the node.
    pub heap_bytes: u64,
    /// Peak heap bytes since last reset.
    pub peak_heap_bytes: u64,
    /// Number of live artifact references held.
    pub artifact_refs: u32,
}

// ---------------------------------------------------------------------------
// LogEntry
// ---------------------------------------------------------------------------

/// A single execution log line from a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Log level.
    pub level: LogLevel,
    /// Log message.
    pub message: String,
}

/// Log severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Debug-level information.
    Debug,
    /// Informational message.
    Info,
    /// Warning.
    Warn,
    /// Error.
    Error,
}

// ---------------------------------------------------------------------------
// ArtifactSummary
// ---------------------------------------------------------------------------

/// A brief summary of a mesh artifact produced or consumed by the node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactSummary {
    /// Artifact ID string.
    pub artifact_id: String,
    /// Human-readable artifact kind (e.g. `"dataset"`, `"model"`, `"report"`).
    pub kind: String,
    /// Byte size, if known.
    pub size_bytes: Option<u64>,
    /// Creation timestamp (ms since epoch).
    pub created_at_ms: u64,
}

// ---------------------------------------------------------------------------
// NodeInspectorData
// ---------------------------------------------------------------------------

/// Full data payload for the node inspector panel.
///
/// Populated by the canvas runtime by merging mesh artifact subscriptions
/// and ActionLog query results for the inspected node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInspectorData {
    /// Node ID.
    pub node_id: String,
    /// Human-readable node label.
    pub label: String,
    /// Node type/kind.
    pub kind: String,
    /// Current node parameters.
    pub parameters: Vec<Parameter>,
    /// Attached tools.
    pub tools: Vec<ToolAttachment>,
    /// Memory snapshot.
    pub memory: Option<MemorySnapshot>,
    /// Recent execution logs (newest first).
    pub logs: Vec<LogEntry>,
    /// Mesh artifacts associated with this node.
    pub artifacts: Vec<ArtifactSummary>,
    /// Arbitrary metadata tags.
    pub metadata: HashMap<String, String>,
}

impl NodeInspectorData {
    /// Create a minimal inspector payload for a node.
    pub fn new(
        node_id: impl Into<String>,
        label: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            node_id: node_id.into(),
            label: label.into(),
            kind: kind.into(),
            parameters: Vec::new(),
            tools: Vec::new(),
            memory: None,
            logs: Vec::new(),
            artifacts: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Add a parameter.
    pub fn with_parameter(mut self, p: Parameter) -> Self {
        self.parameters.push(p);
        self
    }

    /// Add a tool attachment.
    pub fn with_tool(mut self, t: ToolAttachment) -> Self {
        self.tools.push(t);
        self
    }

    /// Set the memory snapshot.
    pub fn with_memory(mut self, m: MemorySnapshot) -> Self {
        self.memory = Some(m);
        self
    }

    /// Append a log entry.
    pub fn with_log(mut self, entry: LogEntry) -> Self {
        self.logs.push(entry);
        self
    }

    /// Add an artifact summary.
    pub fn with_artifact(mut self, a: ArtifactSummary) -> Self {
        self.artifacts.push(a);
        self
    }

    /// Add a metadata tag.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ---------------------------------------------------------------------------
// NodeInspectorStore
// ---------------------------------------------------------------------------

/// In-memory store for inspector data, keyed by node ID.
///
/// The canvas renderer reads from this store to populate the inspector panel.
/// The mesh subscriber writes updates as artifact and ActionLog events arrive.
#[derive(Debug, Default, Clone)]
pub struct NodeInspectorStore {
    data: HashMap<String, NodeInspectorData>,
}

impl NodeInspectorStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace inspector data for a node.
    pub fn upsert(&mut self, data: NodeInspectorData) {
        self.data.insert(data.node_id.clone(), data);
    }

    /// Retrieve inspector data for a node.
    pub fn get(&self, node_id: &str) -> Option<&NodeInspectorData> {
        self.data.get(node_id)
    }

    /// Remove inspector data for a node.
    pub fn remove(&mut self, node_id: &str) {
        self.data.remove(node_id);
    }

    /// Number of nodes currently tracked.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` when the store is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(id: &str) -> NodeInspectorData {
        NodeInspectorData::new(id, "My Node", "http")
            .with_parameter(Parameter::new(
                "url",
                ParameterValue::String("https://example.com".into()),
            ))
            .with_tool(ToolAttachment {
                tool_id: "http".into(),
                display_name: "HTTP".into(),
                active: true,
                config_summary: None,
            })
            .with_memory(MemorySnapshot {
                heap_bytes: 1024,
                peak_heap_bytes: 2048,
                artifact_refs: 3,
            })
            .with_log(LogEntry {
                timestamp_ms: 1_000_000,
                level: LogLevel::Info,
                message: "Node started".into(),
            })
            .with_artifact(ArtifactSummary {
                artifact_id: "art-1".into(),
                kind: "report".into(),
                size_bytes: Some(512),
                created_at_ms: 1_000_100,
            })
    }

    #[test]
    fn inspector_data_builder() {
        let d = make_data("node-1");
        assert_eq!(d.node_id, "node-1");
        assert_eq!(d.parameters.len(), 1);
        assert_eq!(d.tools.len(), 1);
        assert!(d.memory.is_some());
        assert_eq!(d.logs.len(), 1);
        assert_eq!(d.artifacts.len(), 1);
    }

    #[test]
    fn inspector_store_upsert_get() {
        let mut store = NodeInspectorStore::new();
        store.upsert(make_data("n1"));
        store.upsert(make_data("n2"));
        assert_eq!(store.len(), 2);
        assert!(store.get("n1").is_some());
        store.remove("n1");
        assert!(store.get("n1").is_none());
    }

    #[test]
    fn parameter_value_roundtrip() {
        let p = Parameter::new("flag", ParameterValue::Bool(true));
        let json = serde_json::to_string(&p).unwrap();
        let back: Parameter = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
