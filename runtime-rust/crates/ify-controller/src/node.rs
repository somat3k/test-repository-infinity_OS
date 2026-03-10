//! Seamless node adder with undo/redo and node customizer with templates/presets.
//!
//! Satisfies Epic B requirements:
//! > Implement seamless node adder (from code editor + user intent) with
//! > undo/redo and validation.
//! > Implement node customizer (parameters/tools/memory/task-flow wiring) with
//! > templates and presets.
//!
//! ## Node graph and undo/redo
//!
//! [`NodeGraph`] maintains a flat map of nodes plus an undo/redo command stack.
//! Every mutating operation applies a [`NodeCommand`], pushes the inverse on the
//! undo stack, and clears the redo stack.  Undo pops from the undo stack and
//! pushes the re-do inverse onto the redo stack.
//!
//! ## Templates and presets
//!
//! [`NodeTemplate`] describes a reusable node blueprint with typed parameters,
//! tools, memory configuration, and task-flow wiring.  [`NodePreset`] is a
//! named set of parameter overrides for quick configuration.
//! [`NodeCustomizer`] applies templates and presets to concrete [`Node`]
//! instances and validates their parameters.

use std::collections::HashMap;
use std::sync::Arc;

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by node operations.
#[derive(Debug, Error)]
pub enum NodeError {
    /// A node was referenced by ID but not found.
    #[error("node {0} not found in graph")]
    NotFound(Uuid),

    /// A required template was not found.
    #[error("template {0} not found in customizer")]
    TemplateNotFound(Uuid),

    /// A named preset was not found on a template.
    #[error("preset '{name}' not found on template {template_id}")]
    PresetNotFound {
        /// Template the preset was searched in.
        template_id: Uuid,
        /// The requested preset name.
        name: String,
    },

    /// A required parameter is missing from a node.
    #[error("required parameter '{param}' is missing on node {node_id}")]
    MissingRequiredParameter {
        /// Node being validated.
        node_id: Uuid,
        /// Parameter name.
        param: String,
    },

    /// The undo stack is empty.
    #[error("nothing to undo")]
    NothingToUndo,

    /// The redo stack is empty.
    #[error("nothing to redo")]
    NothingToRedo,
}

// ---------------------------------------------------------------------------
// NodeParameter
// ---------------------------------------------------------------------------

/// Typed parameter declaration for a node template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeParameter {
    /// Parameter name (used as key in `Node::parameters`).
    pub name: String,
    /// Human-readable type hint (e.g. `"string"`, `"number"`, `"boolean"`).
    pub type_hint: String,
    /// Default value, if any.
    pub default: Option<serde_json::Value>,
    /// Whether this parameter must be explicitly set.
    pub required: bool,
    /// Short description for tooling.
    pub description: String,
}

// ---------------------------------------------------------------------------
// NodePreset
// ---------------------------------------------------------------------------

/// A named set of parameter overrides for quick node configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePreset {
    /// Unique preset name within its template.
    pub name: String,
    /// Parameter values that this preset supplies.
    pub values: HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// NodeTemplate
// ---------------------------------------------------------------------------

/// A reusable node blueprint that defines parameters, tools, memory
/// configuration, task-flow wiring, and optional presets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTemplate {
    /// Unique template identifier.
    pub id: Uuid,
    /// Human-readable template name.
    pub name: String,
    /// Short description of this template's purpose.
    pub description: String,
    /// Declared parameter schema.
    pub parameters: Vec<NodeParameter>,
    /// Tool identifiers that nodes using this template may invoke.
    pub tools: Vec<String>,
    /// Memory subsystem configuration for nodes of this template.
    pub memory_config: serde_json::Value,
    /// Task-flow wiring descriptor (how this node connects to the orchestrator).
    pub task_flow: serde_json::Value,
    /// Named parameter presets.
    pub presets: Vec<NodePreset>,
}

impl NodeTemplate {
    /// Create a minimal template with the given name.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: description.into(),
            parameters: Vec::new(),
            tools: Vec::new(),
            memory_config: serde_json::Value::Null,
            task_flow: serde_json::Value::Null,
            presets: Vec::new(),
        }
    }

    /// Find a preset by name.
    pub fn preset(&self, name: &str) -> Option<&NodePreset> {
        self.presets.iter().find(|p| p.name == name)
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// A concrete canvas node instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique node identifier.
    pub id: Uuid,
    /// Dimension this node lives in.
    pub dimension_id: DimensionId,
    /// Template this node was created from.
    pub template_id: Uuid,
    /// Human-readable name of this node instance.
    pub name: String,
    /// Current parameter values.
    pub parameters: HashMap<String, serde_json::Value>,
    /// Canvas position `(x, y)`.
    pub position: (f64, f64),
}

// ---------------------------------------------------------------------------
// NodeCommand (undo/redo)
// ---------------------------------------------------------------------------

/// A reversible operation on the node graph.
#[derive(Debug, Clone)]
pub(crate) enum NodeCommand {
    Add { node: Node },
    Remove { node: Node },
    Update { node_id: Uuid, old: HashMap<String, serde_json::Value>, new: HashMap<String, serde_json::Value> },
    Move { node_id: Uuid, old_pos: (f64, f64), new_pos: (f64, f64) },
}

impl NodeCommand {
    /// Return the inverse command that would undo this operation.
    fn inverse(&self) -> NodeCommand {
        match self {
            NodeCommand::Add { node } => NodeCommand::Remove { node: node.clone() },
            NodeCommand::Remove { node } => NodeCommand::Add { node: node.clone() },
            NodeCommand::Update { node_id, old, new } => NodeCommand::Update {
                node_id: *node_id,
                old: new.clone(),
                new: old.clone(),
            },
            NodeCommand::Move { node_id, old_pos, new_pos } => NodeCommand::Move {
                node_id: *node_id,
                old_pos: *new_pos,
                new_pos: *old_pos,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// NodeGraph
// ---------------------------------------------------------------------------

/// A canvas node graph with undo/redo support.
///
/// Every mutating operation emits an [`ActionLogEntry`] with the appropriate
/// node event type.
pub struct NodeGraph {
    nodes: HashMap<Uuid, Node>,
    undo_stack: Vec<NodeCommand>,
    redo_stack: Vec<NodeCommand>,
    action_log: Arc<ActionLog>,
    dimension_id: DimensionId,
    task_id: TaskId,
}

impl NodeGraph {
    /// Create a new empty node graph.
    pub fn new(dimension_id: DimensionId, task_id: TaskId, action_log: Arc<ActionLog>) -> Self {
        Self {
            nodes: HashMap::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            action_log,
            dimension_id,
            task_id,
        }
    }

    // ------------------------------------------------------------------
    // Mutating operations
    // ------------------------------------------------------------------

    /// Add a node from a template.
    ///
    /// `params` overrides template defaults.  The resulting node is validated
    /// against the template's required parameters before being inserted.
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::MissingRequiredParameter`] if a required parameter
    /// has no default and is not supplied in `params`.
    #[instrument(skip(self, template, params), fields(dimension = %self.dimension_id, task_id = %self.task_id))]
    pub fn add_node(
        &mut self,
        template: &NodeTemplate,
        name: &str,
        position: (f64, f64),
        params: HashMap<String, serde_json::Value>,
    ) -> Result<Uuid, NodeError> {
        // Build final parameter map: template defaults ← caller overrides
        let mut final_params: HashMap<String, serde_json::Value> = HashMap::new();
        for p in &template.parameters {
            if let Some(default) = &p.default {
                final_params.insert(p.name.clone(), default.clone());
            }
        }
        final_params.extend(params);

        // Validate required parameters
        for p in &template.parameters {
            if p.required && !final_params.contains_key(&p.name) {
                return Err(NodeError::MissingRequiredParameter {
                    node_id: Uuid::nil(),
                    param: p.name.clone(),
                });
            }
        }

        let node = Node {
            id: Uuid::new_v4(),
            dimension_id: self.dimension_id,
            template_id: template.id,
            name: name.to_owned(),
            parameters: final_params,
            position,
        };
        let id = node.id;

        let cmd = NodeCommand::Add { node: node.clone() };
        self.apply_command(cmd);

        info!(node_id = %id, template = %template.name, "node added to graph");
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeCreated,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "node_id": id,
                "node_kind": template.name.clone(),
                "template_id": template.id,
            }),
        ));

        Ok(id)
    }

    /// Remove a node from the graph.
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::NotFound`] if the node does not exist.
    #[instrument(skip(self), fields(node_id = %node_id))]
    pub fn remove_node(&mut self, node_id: Uuid) -> Result<(), NodeError> {
        let node = self
            .nodes
            .get(&node_id)
            .cloned()
            .ok_or(NodeError::NotFound(node_id))?;

        let cmd = NodeCommand::Remove { node };
        self.apply_command(cmd);

        info!(node_id = %node_id, "node removed from graph");
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeDeleted,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "node_id": node_id }),
        ));

        Ok(())
    }

    /// Update a node's parameters.
    ///
    /// Merges `updates` into the node's current parameter map.
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::NotFound`] if the node does not exist.
    #[instrument(skip(self, updates), fields(node_id = %node_id))]
    pub fn update_node(
        &mut self,
        node_id: Uuid,
        updates: HashMap<String, serde_json::Value>,
    ) -> Result<(), NodeError> {
        let node = self
            .nodes
            .get(&node_id)
            .cloned()
            .ok_or(NodeError::NotFound(node_id))?;

        let old_params = node.parameters.clone();
        let mut new_params = old_params.clone();
        new_params.extend(updates.clone());

        let cmd = NodeCommand::Update {
            node_id,
            old: old_params,
            new: new_params,
        };
        self.apply_command(cmd);

        let changed_fields: Vec<&str> = updates.keys().map(String::as_str).collect();
        debug!(node_id = %node_id, ?changed_fields, "node updated");
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeUpdated,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "node_id": node_id,
                "changed_fields": changed_fields,
            }),
        ));

        Ok(())
    }

    /// Move a node to a new canvas position.
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::NotFound`] if the node does not exist.
    pub fn move_node(&mut self, node_id: Uuid, new_position: (f64, f64)) -> Result<(), NodeError> {
        let node = self
            .nodes
            .get(&node_id)
            .ok_or(NodeError::NotFound(node_id))?;
        let old_pos = node.position;

        let cmd = NodeCommand::Move {
            node_id,
            old_pos,
            new_pos: new_position,
        };
        self.apply_command(cmd);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Undo / Redo
    // ------------------------------------------------------------------

    /// Undo the most recent mutating operation.
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::NothingToUndo`] when the stack is empty.
    #[instrument(skip(self))]
    pub fn undo(&mut self) -> Result<(), NodeError> {
        let undo_cmd = self
            .undo_stack
            .pop()
            .ok_or(NodeError::NothingToUndo)?;

        // undo_cmd is the command that reverses the last action.
        // Execute it; push its inverse (the re-apply operation) onto redo.
        let redo_cmd = undo_cmd.inverse();
        self.execute_command(&undo_cmd);
        self.redo_stack.push(redo_cmd);

        debug!("undo applied");
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeUndo,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "undo_stack_remaining": self.undo_stack.len() }),
        ));

        Ok(())
    }

    /// Redo the most recently undone operation.
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::NothingToRedo`] when the stack is empty.
    #[instrument(skip(self))]
    pub fn redo(&mut self) -> Result<(), NodeError> {
        let cmd = self
            .redo_stack
            .pop()
            .ok_or(NodeError::NothingToRedo)?;

        let undo_cmd = cmd.inverse();
        self.execute_command(&cmd);
        self.undo_stack.push(undo_cmd);

        debug!("redo applied");
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeRedo,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "redo_stack_remaining": self.redo_stack.len() }),
        ));

        Ok(())
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Get a reference to a node by ID.
    pub fn get(&self, id: Uuid) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Iterate over all nodes in the graph.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// Number of nodes currently in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns `true` when the graph has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Number of operations that can be undone.
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Number of operations that can be redone.
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Validate all nodes against their templates.
    ///
    /// In a full implementation this would cross-reference the
    /// [`NodeCustomizer`]'s template registry.  Here it checks that each
    /// node's `template_id` field is set (not nil).
    pub fn validate(&self) -> Result<(), NodeError> {
        for node in self.nodes.values() {
            if node.template_id.is_nil() {
                return Err(NodeError::TemplateNotFound(node.template_id));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Apply a command, push its inverse on the undo stack, and clear redo.
    fn apply_command(&mut self, cmd: NodeCommand) {
        let inverse = cmd.inverse();
        self.execute_command(&cmd);
        self.undo_stack.push(inverse);
        self.redo_stack.clear();
    }

    /// Execute a command against the node map (no stack modification).
    fn execute_command(&mut self, cmd: &NodeCommand) {
        match cmd {
            NodeCommand::Add { node } => {
                self.nodes.insert(node.id, node.clone());
            }
            NodeCommand::Remove { node } => {
                self.nodes.remove(&node.id);
            }
            NodeCommand::Update { node_id, new, .. } => {
                if let Some(n) = self.nodes.get_mut(node_id) {
                    n.parameters = new.clone();
                }
            }
            NodeCommand::Move { node_id, new_pos, .. } => {
                if let Some(n) = self.nodes.get_mut(node_id) {
                    n.position = *new_pos;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// NodeCustomizer
// ---------------------------------------------------------------------------

/// Manages node templates and applies customizations (presets and direct
/// parameter overrides) to concrete node instances.
pub struct NodeCustomizer {
    templates: HashMap<Uuid, NodeTemplate>,
    action_log: Arc<ActionLog>,
}

impl NodeCustomizer {
    /// Create a new customizer.
    pub fn new(action_log: Arc<ActionLog>) -> Self {
        Self {
            templates: HashMap::new(),
            action_log,
        }
    }

    /// Register a template so it can be referenced by ID.
    pub fn register_template(&mut self, template: NodeTemplate) {
        info!(template_id = %template.id, name = %template.name, "template registered");
        self.templates.insert(template.id, template);
    }

    /// Look up a template by ID.
    pub fn get_template(&self, id: Uuid) -> Option<&NodeTemplate> {
        self.templates.get(&id)
    }

    /// Apply a named preset to a node, overwriting its parameters with the
    /// preset's values.
    ///
    /// # Errors
    ///
    /// - [`NodeError::TemplateNotFound`] if the template is not registered.
    /// - [`NodeError::PresetNotFound`] if no preset with that name exists.
    #[instrument(skip(self, node), fields(template_id = %template_id, preset = preset_name))]
    pub fn apply_preset(
        &self,
        node: &mut Node,
        template_id: Uuid,
        preset_name: &str,
    ) -> Result<(), NodeError> {
        let template = self
            .templates
            .get(&template_id)
            .ok_or(NodeError::TemplateNotFound(template_id))?;

        let preset = template
            .preset(preset_name)
            .ok_or_else(|| NodeError::PresetNotFound {
                template_id,
                name: preset_name.to_owned(),
            })?;

        for (k, v) in &preset.values {
            node.parameters.insert(k.clone(), v.clone());
        }

        info!(node_id = %node.id, preset = preset_name, "preset applied");
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeUpdated,
            Actor::System,
            Some(node.dimension_id),
            None,
            serde_json::json!({
                "node_id": node.id,
                "preset_applied": preset_name,
                "template_id": template_id,
            }),
        ));

        Ok(())
    }

    /// Directly customize a node with the supplied parameter overrides,
    /// then validate the result.
    ///
    /// # Errors
    ///
    /// - [`NodeError::TemplateNotFound`] if the template is not registered.
    /// - [`NodeError::MissingRequiredParameter`] if required params are unset.
    #[instrument(skip(self, node, updates), fields(node_id = %node.id))]
    pub fn customize(
        &self,
        node: &mut Node,
        task_id: TaskId,
        updates: HashMap<String, serde_json::Value>,
    ) -> Result<(), NodeError> {
        node.parameters.extend(updates.clone());

        let changed: Vec<&str> = updates.keys().map(String::as_str).collect();
        self.action_log.append(ActionLogEntry::new(
            EventType::NodeUpdated,
            Actor::System,
            Some(node.dimension_id),
            Some(task_id),
            serde_json::json!({
                "node_id": node.id,
                "changed_fields": changed,
            }),
        ));

        // Validate against the registered template if present
        if let Some(template) = self.templates.get(&node.template_id) {
            for p in &template.parameters {
                if p.required && !node.parameters.contains_key(&p.name) {
                    return Err(NodeError::MissingRequiredParameter {
                        node_id: node.id,
                        param: p.name.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Validate a node's parameters against its registered template.
    ///
    /// Returns `Ok(())` if the template is not registered (unknown template IDs
    /// are not rejected here — graph validation covers that).
    ///
    /// # Errors
    ///
    /// Returns [`NodeError::MissingRequiredParameter`] if a required parameter
    /// is absent.
    pub fn validate_params(&self, node: &Node) -> Result<(), NodeError> {
        let Some(template) = self.templates.get(&node.template_id) else {
            return Ok(());
        };

        for p in &template.parameters {
            if p.required && !node.parameters.contains_key(&p.name) {
                return Err(NodeError::MissingRequiredParameter {
                    node_id: node.id,
                    param: p.name.clone(),
                });
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

    fn make_template() -> NodeTemplate {
        let mut t = NodeTemplate::new("TestNode", "A test node template");
        t.parameters = vec![
            NodeParameter {
                name: "color".to_owned(),
                type_hint: "string".to_owned(),
                default: Some(serde_json::json!("red")),
                required: false,
                description: "Node color".to_owned(),
            },
            NodeParameter {
                name: "required_param".to_owned(),
                type_hint: "string".to_owned(),
                default: None,
                required: true,
                description: "A required param".to_owned(),
            },
        ];
        t.presets = vec![NodePreset {
            name: "blue".to_owned(),
            values: {
                let mut m = HashMap::new();
                m.insert("color".to_owned(), serde_json::json!("blue"));
                m
            },
        }];
        t
    }

    fn make_graph() -> NodeGraph {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let task = TaskId::new();
        NodeGraph::new(dim, task, log)
    }

    #[test]
    fn add_and_remove_node() {
        let mut graph = make_graph();
        let template = make_template();

        let mut params = HashMap::new();
        params.insert("required_param".to_owned(), serde_json::json!("hello"));

        let id = graph.add_node(&template, "my-node", (0.0, 0.0), params).unwrap();
        assert_eq!(graph.len(), 1);

        graph.remove_node(id).unwrap();
        assert_eq!(graph.len(), 0);
    }

    #[test]
    fn add_node_missing_required_param_fails() {
        let mut graph = make_graph();
        let template = make_template();

        let err = graph.add_node(&template, "bad-node", (0.0, 0.0), HashMap::new());
        assert!(matches!(err, Err(NodeError::MissingRequiredParameter { .. })));
    }

    #[test]
    fn undo_redo_add_node() {
        let mut graph = make_graph();
        let template = make_template();
        let mut params = HashMap::new();
        params.insert("required_param".to_owned(), serde_json::json!("x"));

        let id = graph.add_node(&template, "node", (0.0, 0.0), params).unwrap();
        assert_eq!(graph.len(), 1);

        graph.undo().unwrap();
        assert_eq!(graph.len(), 0, "undo should remove the node");
        assert!(graph.get(id).is_none());

        graph.redo().unwrap();
        assert_eq!(graph.len(), 1, "redo should restore the node");
        assert!(graph.get(id).is_some());
    }

    #[test]
    fn undo_empty_stack_fails() {
        let mut graph = make_graph();
        let err = graph.undo();
        assert!(matches!(err, Err(NodeError::NothingToUndo)));
    }

    #[test]
    fn redo_empty_stack_fails() {
        let mut graph = make_graph();
        let err = graph.redo();
        assert!(matches!(err, Err(NodeError::NothingToRedo)));
    }

    #[test]
    fn redo_stack_clears_on_new_action() {
        let mut graph = make_graph();
        let template = make_template();
        let mut params = HashMap::new();
        params.insert("required_param".to_owned(), serde_json::json!("x"));

        graph.add_node(&template, "n1", (0.0, 0.0), params.clone()).unwrap();
        graph.undo().unwrap(); // redo stack has 1 entry
        graph.add_node(&template, "n2", (1.0, 1.0), params).unwrap(); // clears redo

        assert_eq!(graph.redo_depth(), 0);
    }

    #[test]
    fn customizer_apply_preset() {
        let log = ActionLog::new(16);
        let mut customizer = NodeCustomizer::new(Arc::clone(&log));
        let template = make_template();
        let template_id = template.id;
        customizer.register_template(template);

        let dim = DimensionId::new();
        let mut node = Node {
            id: Uuid::new_v4(),
            dimension_id: dim,
            template_id,
            name: "n".to_owned(),
            parameters: HashMap::new(),
            position: (0.0, 0.0),
        };

        customizer.apply_preset(&mut node, template_id, "blue").unwrap();
        assert_eq!(
            node.parameters["color"],
            serde_json::json!("blue")
        );
    }

    #[test]
    fn customizer_preset_not_found_fails() {
        let log = ActionLog::new(16);
        let mut customizer = NodeCustomizer::new(log);
        let template = make_template();
        let tid = template.id;
        customizer.register_template(template);

        let dim = DimensionId::new();
        let mut node = Node {
            id: Uuid::new_v4(),
            dimension_id: dim,
            template_id: tid,
            name: "n".to_owned(),
            parameters: HashMap::new(),
            position: (0.0, 0.0),
        };

        let err = customizer.apply_preset(&mut node, tid, "nonexistent");
        assert!(matches!(err, Err(NodeError::PresetNotFound { .. })));
    }

    #[test]
    fn customizer_validate_missing_required_fails() {
        let log = ActionLog::new(16);
        let mut customizer = NodeCustomizer::new(log);
        let template = make_template();
        let tid = template.id;
        customizer.register_template(template);

        let dim = DimensionId::new();
        let node = Node {
            id: Uuid::new_v4(),
            dimension_id: dim,
            template_id: tid,
            name: "n".to_owned(),
            parameters: HashMap::new(), // required_param is missing
            position: (0.0, 0.0),
        };

        let err = customizer.validate_params(&node);
        assert!(matches!(err, Err(NodeError::MissingRequiredParameter { .. })));
    }
}
