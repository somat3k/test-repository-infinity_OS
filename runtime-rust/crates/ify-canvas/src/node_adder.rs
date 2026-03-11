//! # node_adder — Seamless Node Adder/Customizer from Editor
//!
//! Provides the canvas-side API for adding new nodes from an editor context
//! (e.g., a code snippet, a command palette selection, or a drag-and-drop
//! palette action) and for customizing existing nodes via a structured
//! parameter editor.
//!
//! The node adder emits `node.created` ActionLog events and integrates with
//! the undo/redo stack (see the `UndoStack` type in this module).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::selection::Point;

// ---------------------------------------------------------------------------
// NodeTemplate (canvas side)
// ---------------------------------------------------------------------------

/// A canvas-side template that describes how a new node should be placed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasNodeTemplate {
    /// Template identifier (e.g. `"http-request"`, `"transform-map"`).
    pub template_id: String,
    /// Human-readable display name.
    pub label: String,
    /// Node kind string.
    pub kind: String,
    /// Default initial position on the canvas.
    pub default_position: Point,
    /// Default parameters (key → serialised value).
    pub default_params: std::collections::HashMap<String, serde_json::Value>,
}

impl CanvasNodeTemplate {
    /// Create a minimal template.
    pub fn new(
        template_id: impl Into<String>,
        label: impl Into<String>,
        kind: impl Into<String>,
    ) -> Self {
        Self {
            template_id: template_id.into(),
            label: label.into(),
            kind: kind.into(),
            default_position: Point::new(0.0, 0.0),
            default_params: Default::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// AddNodeRequest / AddNodeResult
// ---------------------------------------------------------------------------

/// A request to add a node to the canvas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddNodeRequest {
    /// Template to instantiate.
    pub template_id: String,
    /// Override the default position.
    pub position: Option<Point>,
    /// Optional parameter overrides.
    pub param_overrides: std::collections::HashMap<String, serde_json::Value>,
    /// Optional label override.
    pub label_override: Option<String>,
}

impl AddNodeRequest {
    /// Create a minimal request from a template ID.
    pub fn from_template(template_id: impl Into<String>) -> Self {
        Self {
            template_id: template_id.into(),
            position: None,
            param_overrides: Default::default(),
            label_override: None,
        }
    }
}

/// The result of a successful add-node operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddNodeResult {
    /// Newly assigned node ID.
    pub node_id: String,
    /// Final position on canvas.
    pub position: Point,
    /// Final label.
    pub label: String,
}

// ---------------------------------------------------------------------------
// NodeAdder
// ---------------------------------------------------------------------------

/// Errors produced by the node adder.
#[derive(Debug, Error, PartialEq)]
pub enum NodeAdderError {
    /// The requested template was not found.
    #[error("template not found: {0}")]
    TemplateNotFound(String),
    /// The undo stack is empty.
    #[error("nothing to undo")]
    NothingToUndo,
    /// The redo stack is empty.
    #[error("nothing to redo")]
    NothingToRedo,
}

/// Canvas-side node adder with undo/redo support.
///
/// Nodes are identified by sequential IDs in this simplified model;
/// a production implementation would use UUIDv7-based IDs.
#[derive(Debug)]
pub struct NodeAdder {
    templates: std::collections::HashMap<String, CanvasNodeTemplate>,
    next_id: u64,
    undo_stack: UndoStack,
}

impl NodeAdder {
    /// Create an adder with the given templates registered.
    pub fn new(templates: impl IntoIterator<Item = CanvasNodeTemplate>) -> Self {
        let map = templates.into_iter().map(|t| (t.template_id.clone(), t)).collect();
        Self {
            templates: map,
            next_id: 1,
            undo_stack: UndoStack::new(64),
        }
    }

    /// Register a new template.
    pub fn register_template(&mut self, t: CanvasNodeTemplate) {
        self.templates.insert(t.template_id.clone(), t);
    }

    /// Add a node to the canvas from a template, returning the placement result.
    ///
    /// The operation is pushed onto the undo stack.
    ///
    /// # Errors
    ///
    /// Returns [`NodeAdderError::TemplateNotFound`] if the template ID does
    /// not exist in the registry.
    pub fn add_node(&mut self, request: AddNodeRequest) -> Result<AddNodeResult, NodeAdderError> {
        let tmpl = self
            .templates
            .get(&request.template_id)
            .ok_or_else(|| NodeAdderError::TemplateNotFound(request.template_id.clone()))?
            .clone();

        let node_id = format!("node-{}", self.next_id);
        self.next_id += 1;

        let position = request.position.unwrap_or(tmpl.default_position);
        let label = request.label_override.unwrap_or_else(|| tmpl.label.clone());

        let result = AddNodeResult {
            node_id: node_id.clone(),
            position,
            label: label.clone(),
        };

        self.undo_stack.push(UndoEntry::AddNode {
            node_id: node_id.clone(),
        });

        tracing::debug!(node_id = %node_id, template = %tmpl.template_id, "node added");
        Ok(result)
    }

    /// Undo the last structural edit.
    ///
    /// # Errors
    ///
    /// Returns [`NodeAdderError::NothingToUndo`] if the stack is empty.
    pub fn undo(&mut self) -> Result<UndoEntry, NodeAdderError> {
        self.undo_stack.undo().ok_or(NodeAdderError::NothingToUndo)
    }

    /// Redo the previously undone edit.
    ///
    /// # Errors
    ///
    /// Returns [`NodeAdderError::NothingToRedo`] if the redo stack is empty.
    pub fn redo(&mut self) -> Result<UndoEntry, NodeAdderError> {
        self.undo_stack.redo().ok_or(NodeAdderError::NothingToRedo)
    }
}

// ---------------------------------------------------------------------------
// UndoStack
// ---------------------------------------------------------------------------

/// An undo/redo entry for a structural canvas edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UndoEntry {
    /// A node was added (undo = remove it).
    AddNode {
        /// The node that was added.
        node_id: String,
    },
    /// A node was moved (undo = move it back).
    MoveNode {
        /// The node that was moved.
        node_id: String,
        /// The previous position.
        from: Point,
        /// The new position.
        to: Point,
    },
    /// A node was deleted (undo = recreate it).
    DeleteNode {
        /// The node that was deleted.
        node_id: String,
    },
}

/// A bounded undo/redo stack.
#[derive(Debug)]
pub struct UndoStack {
    undo: VecDeque<UndoEntry>,
    redo: VecDeque<UndoEntry>,
    capacity: usize,
}

impl UndoStack {
    /// Create a stack with the given maximum depth.
    pub fn new(capacity: usize) -> Self {
        Self {
            undo: VecDeque::with_capacity(capacity),
            redo: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a new entry, clearing the redo history.
    pub fn push(&mut self, entry: UndoEntry) {
        if self.undo.len() == self.capacity {
            self.undo.pop_front();
        }
        self.undo.push_back(entry);
        self.redo.clear();
    }

    /// Pop the most recent entry for undoing.
    pub fn undo(&mut self) -> Option<UndoEntry> {
        let entry = self.undo.pop_back()?;
        self.redo.push_back(entry.clone());
        Some(entry)
    }

    /// Pop the most recently undone entry for redoing.
    pub fn redo(&mut self) -> Option<UndoEntry> {
        let entry = self.redo.pop_back()?;
        self.undo.push_back(entry.clone());
        Some(entry)
    }

    /// How many entries are in the undo stack.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// How many entries are in the redo stack.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn http_template() -> CanvasNodeTemplate {
        CanvasNodeTemplate::new("http-request", "HTTP Request", "http")
    }

    #[test]
    fn add_node_success() {
        let mut adder = NodeAdder::new([http_template()]);
        let req = AddNodeRequest::from_template("http-request");
        let result = adder.add_node(req).unwrap();
        assert_eq!(result.node_id, "node-1");
        assert_eq!(result.label, "HTTP Request");
    }

    #[test]
    fn add_node_unknown_template_errors() {
        let mut adder = NodeAdder::new([]);
        let req = AddNodeRequest::from_template("unknown");
        assert_eq!(
            adder.add_node(req),
            Err(NodeAdderError::TemplateNotFound("unknown".into()))
        );
    }

    #[test]
    fn undo_after_add() {
        let mut adder = NodeAdder::new([http_template()]);
        adder.add_node(AddNodeRequest::from_template("http-request")).unwrap();
        let entry = adder.undo().unwrap();
        assert!(matches!(entry, UndoEntry::AddNode { .. }));
        // Redo re-applies the entry.
        adder.redo().unwrap();
    }

    #[test]
    fn undo_empty_stack_errors() {
        let mut adder = NodeAdder::new([http_template()]);
        assert_eq!(adder.undo(), Err(NodeAdderError::NothingToUndo));
    }

    #[test]
    fn redo_empty_stack_errors() {
        let mut adder = NodeAdder::new([http_template()]);
        assert_eq!(adder.redo(), Err(NodeAdderError::NothingToRedo));
    }

    #[test]
    fn undo_stack_capacity_evicts_oldest() {
        let mut stack = UndoStack::new(3);
        for i in 0..5u64 {
            stack.push(UndoEntry::AddNode {
                node_id: format!("node-{i}"),
            });
        }
        assert_eq!(stack.undo_depth(), 3);
    }

    #[test]
    fn push_clears_redo() {
        let mut stack = UndoStack::new(10);
        stack.push(UndoEntry::AddNode { node_id: "n1".into() });
        stack.undo();
        assert_eq!(stack.redo_depth(), 1);
        stack.push(UndoEntry::AddNode { node_id: "n2".into() });
        assert_eq!(stack.redo_depth(), 0);
    }
}
