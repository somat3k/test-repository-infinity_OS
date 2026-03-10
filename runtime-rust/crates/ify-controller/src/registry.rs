//! Block registration pipeline: editor instance → interpreter attach → runtime bind.
//!
//! Satisfies Epic B requirement:
//! > Implement block registration pipeline: create new editor instance, attach
//! > interpreter, bind to runtime.
//!
//! ## Pipeline stages
//!
//! ```text
//! register_block()
//!       │
//!       ▼
//! create_editor(block_id, language)
//!       │
//!       ▼
//! attach_interpreter(block_id, interpreter_type, config)
//!       │
//!       ▼
//! bind_runtime(block_id) ──► RuntimeBinding
//! ```
//!
//! Each stage emits an [`ActionLogEntry`] and validates that the block exists
//! in the correct dimension before proceeding.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the block registry.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// The block ID is not known to this registry.
    #[error("block {0} not found in registry")]
    BlockNotFound(Uuid),

    /// A required pipeline stage has not been completed yet.
    #[error("block {id} is not ready for this operation: {reason}")]
    StageNotComplete {
        /// Block identifier.
        id: Uuid,
        /// Human-readable explanation of which stage is missing.
        reason: &'static str,
    },

    /// An editor was created for a block that already has one.
    #[error("block {0} already has an editor instance")]
    EditorAlreadyExists(Uuid),

    /// An interpreter was attached to a block that already has one.
    #[error("block {0} already has an interpreter attached")]
    InterpreterAlreadyAttached(Uuid),

    /// An attempt to bind the runtime when a binding already exists.
    #[error("block {0} is already bound to the runtime")]
    AlreadyBound(Uuid),
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// A live editor instance associated with a registered block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorInstance {
    /// Unique identifier for this editor session.
    pub id: Uuid,
    /// Dimension this editor belongs to.
    pub dimension_id: DimensionId,
    /// Language/MIME type for this editor (e.g. `"rust"`, `"python"`).
    pub language: String,
    /// Initial content buffer (may be empty).
    pub content: String,
}

/// An interpreter attached to an editor instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterpreterAttachment {
    /// The editor this interpreter is attached to.
    pub editor_id: Uuid,
    /// Interpreter type identifier (e.g. `"lsp"`, `"jupyter"`, `"tree-sitter"`).
    pub interpreter_type: String,
    /// Interpreter-specific configuration.
    pub config: serde_json::Value,
}

/// A runtime binding confirming the block is fully wired and ready for
/// task submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBinding {
    /// The block this binding covers.
    pub block_id: Uuid,
    /// The editor associated with this block.
    pub editor_id: Uuid,
    /// Dimension this binding is scoped to.
    pub dimension_id: DimensionId,
    /// Task that owns this registration.
    pub task_id: TaskId,
}

// ---------------------------------------------------------------------------
// Internal block record
// ---------------------------------------------------------------------------

struct RegisteredBlock {
    dimension_id: DimensionId,
    task_id: TaskId,
    editor: Option<EditorInstance>,
    interpreter: Option<InterpreterAttachment>,
    binding: Option<RuntimeBinding>,
}

// ---------------------------------------------------------------------------
// BlockRegistry
// ---------------------------------------------------------------------------

/// Registry that owns the block registration pipeline.
///
/// All methods are thread-safe via internal locking.
pub struct BlockRegistry {
    blocks: Mutex<HashMap<Uuid, RegisteredBlock>>,
    action_log: Arc<ActionLog>,
}

impl std::fmt::Debug for BlockRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.blocks.lock().map(|g| g.len()).unwrap_or(0);
        write!(f, "BlockRegistry {{ blocks: {len} }}")
    }
}

impl BlockRegistry {
    /// Create a new empty registry.
    pub fn new(action_log: Arc<ActionLog>) -> Self {
        Self {
            blocks: Mutex::new(HashMap::new()),
            action_log,
        }
    }

    /// **Stage 1** — Register a new block and return its ID.
    ///
    /// Blocks must be registered before any other pipeline stage.
    #[instrument(skip(self), fields(dimension = %dimension_id, task_id = %task_id))]
    pub fn register_block(
        &self,
        dimension_id: DimensionId,
        task_id: TaskId,
    ) -> Uuid {
        let id = Uuid::new_v4();
        {
            let mut blocks = self.blocks.lock().expect("registry lock poisoned");
            blocks.insert(
                id,
                RegisteredBlock {
                    dimension_id,
                    task_id,
                    editor: None,
                    interpreter: None,
                    binding: None,
                },
            );
        }

        info!(block_id = %id, "block registered");
        self.action_log.append(ActionLogEntry::new(
            EventType::ControllerRegistered,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "block_id": id,
                "stage": "register",
            }),
        ));

        id
    }

    /// **Stage 2** — Create an editor instance for the given block.
    ///
    /// # Errors
    ///
    /// - [`RegistryError::BlockNotFound`] if the block is unknown.
    /// - [`RegistryError::EditorAlreadyExists`] if an editor already exists.
    #[instrument(skip(self), fields(block_id = %block_id))]
    pub fn create_editor(
        &self,
        block_id: Uuid,
        language: &str,
    ) -> Result<Uuid, RegistryError> {
        let mut blocks = self.blocks.lock().expect("registry lock poisoned");
        let block = blocks
            .get_mut(&block_id)
            .ok_or(RegistryError::BlockNotFound(block_id))?;

        if block.editor.is_some() {
            return Err(RegistryError::EditorAlreadyExists(block_id));
        }

        let editor_id = Uuid::new_v4();
        let editor = EditorInstance {
            id: editor_id,
            dimension_id: block.dimension_id,
            language: language.to_owned(),
            content: String::new(),
        };

        let dimension_id = block.dimension_id;
        let task_id = block.task_id;
        block.editor = Some(editor);

        debug!(block_id = %block_id, editor_id = %editor_id, "editor created");
        self.action_log.append(ActionLogEntry::new(
            EventType::EditorCreated,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "block_id": block_id,
                "editor_id": editor_id,
                "language": language,
            }),
        ));

        Ok(editor_id)
    }

    /// **Stage 3** — Attach an interpreter to the block's editor.
    ///
    /// # Errors
    ///
    /// - [`RegistryError::BlockNotFound`] if the block is unknown.
    /// - [`RegistryError::StageNotComplete`] if no editor has been created yet.
    /// - [`RegistryError::InterpreterAlreadyAttached`] if one already exists.
    #[instrument(skip(self, config), fields(block_id = %block_id))]
    pub fn attach_interpreter(
        &self,
        block_id: Uuid,
        interpreter_type: &str,
        config: serde_json::Value,
    ) -> Result<(), RegistryError> {
        let mut blocks = self.blocks.lock().expect("registry lock poisoned");
        let block = blocks
            .get_mut(&block_id)
            .ok_or(RegistryError::BlockNotFound(block_id))?;

        let editor_id = block
            .editor
            .as_ref()
            .map(|e| e.id)
            .ok_or(RegistryError::StageNotComplete {
                id: block_id,
                reason: "create_editor() must be called before attach_interpreter()",
            })?;

        if block.interpreter.is_some() {
            return Err(RegistryError::InterpreterAlreadyAttached(block_id));
        }

        let attachment = InterpreterAttachment {
            editor_id,
            interpreter_type: interpreter_type.to_owned(),
            config: config.clone(),
        };

        let dimension_id = block.dimension_id;
        let task_id = block.task_id;
        block.interpreter = Some(attachment);

        info!(block_id = %block_id, interpreter = interpreter_type, "interpreter attached");
        self.action_log.append(ActionLogEntry::new(
            EventType::InterpreterAttached,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "block_id": block_id,
                "editor_id": editor_id,
                "interpreter_type": interpreter_type,
                "config": config,
            }),
        ));

        Ok(())
    }

    /// **Stage 4** — Bind the block to the executor runtime.
    ///
    /// Returns a [`RuntimeBinding`] confirming the block is fully wired.
    ///
    /// # Errors
    ///
    /// - [`RegistryError::BlockNotFound`] if the block is unknown.
    /// - [`RegistryError::StageNotComplete`] if the interpreter has not been attached.
    /// - [`RegistryError::AlreadyBound`] if this block was already bound.
    #[instrument(skip(self), fields(block_id = %block_id))]
    pub fn bind_runtime(&self, block_id: Uuid) -> Result<RuntimeBinding, RegistryError> {
        let mut blocks = self.blocks.lock().expect("registry lock poisoned");
        let block = blocks
            .get_mut(&block_id)
            .ok_or(RegistryError::BlockNotFound(block_id))?;

        let editor_id = block
            .editor
            .as_ref()
            .map(|e| e.id)
            .ok_or(RegistryError::StageNotComplete {
                id: block_id,
                reason: "create_editor() and attach_interpreter() must be called first",
            })?;

        block.interpreter.as_ref().ok_or(RegistryError::StageNotComplete {
            id: block_id,
            reason: "attach_interpreter() must be called before bind_runtime()",
        })?;

        if block.binding.is_some() {
            return Err(RegistryError::AlreadyBound(block_id));
        }

        let binding = RuntimeBinding {
            block_id,
            editor_id,
            dimension_id: block.dimension_id,
            task_id: block.task_id,
        };

        let dimension_id = block.dimension_id;
        let task_id = block.task_id;
        block.binding = Some(binding.clone());

        info!(block_id = %block_id, "block bound to runtime");
        self.action_log.append(ActionLogEntry::new(
            EventType::RuntimeBound,
            Actor::System,
            Some(dimension_id),
            Some(task_id),
            serde_json::json!({
                "block_id": block_id,
                "editor_id": editor_id,
                "dimension_id": dimension_id.to_string(),
            }),
        ));

        Ok(binding)
    }

    /// Return the [`RuntimeBinding`] for a block, if it has been bound.
    pub fn binding_for(&self, block_id: Uuid) -> Option<RuntimeBinding> {
        self.blocks
            .lock()
            .expect("registry lock poisoned")
            .get(&block_id)
            .and_then(|b| b.binding.clone())
    }

    /// Return the [`EditorInstance`] for a block, if one has been created.
    pub fn editor_for(&self, block_id: Uuid) -> Option<EditorInstance> {
        self.blocks
            .lock()
            .expect("registry lock poisoned")
            .get(&block_id)
            .and_then(|b| b.editor.clone())
    }

    /// Number of blocks currently in the registry.
    pub fn len(&self) -> usize {
        self.blocks.lock().expect("registry lock poisoned").len()
    }

    /// Returns `true` when no blocks are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> (BlockRegistry, Arc<ActionLog>) {
        let log = ActionLog::new(32);
        let reg = BlockRegistry::new(Arc::clone(&log));
        (reg, log)
    }

    #[test]
    fn full_pipeline_succeeds() {
        let (reg, log) = make_registry();
        let dim = DimensionId::new();
        let task = TaskId::new();

        let block_id = reg.register_block(dim, task);
        let editor_id = reg.create_editor(block_id, "rust").unwrap();
        reg.attach_interpreter(block_id, "lsp", serde_json::json!({}))
            .unwrap();
        let binding = reg.bind_runtime(block_id).unwrap();

        assert_eq!(binding.block_id, block_id);
        assert_eq!(binding.editor_id, editor_id);
        assert_eq!(binding.dimension_id, dim);

        // register + editor + interpreter + bind = 4 log entries
        assert_eq!(log.len(), 4);
    }

    #[test]
    fn create_editor_unknown_block_fails() {
        let (reg, _) = make_registry();
        let err = reg.create_editor(Uuid::new_v4(), "rust");
        assert!(matches!(err, Err(RegistryError::BlockNotFound(_))));
    }

    #[test]
    fn attach_without_editor_fails() {
        let (reg, _) = make_registry();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let block_id = reg.register_block(dim, task);

        let err = reg.attach_interpreter(block_id, "lsp", serde_json::json!({}));
        assert!(matches!(err, Err(RegistryError::StageNotComplete { .. })));
    }

    #[test]
    fn bind_without_interpreter_fails() {
        let (reg, _) = make_registry();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let block_id = reg.register_block(dim, task);
        reg.create_editor(block_id, "python").unwrap();

        let err = reg.bind_runtime(block_id);
        assert!(matches!(err, Err(RegistryError::StageNotComplete { .. })));
    }

    #[test]
    fn duplicate_editor_fails() {
        let (reg, _) = make_registry();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let block_id = reg.register_block(dim, task);

        reg.create_editor(block_id, "rust").unwrap();
        let err = reg.create_editor(block_id, "rust");
        assert!(matches!(err, Err(RegistryError::EditorAlreadyExists(_))));
    }

    #[test]
    fn duplicate_bind_fails() {
        let (reg, _) = make_registry();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let block_id = reg.register_block(dim, task);

        reg.create_editor(block_id, "rust").unwrap();
        reg.attach_interpreter(block_id, "lsp", serde_json::json!({}))
            .unwrap();
        reg.bind_runtime(block_id).unwrap();

        let err = reg.bind_runtime(block_id);
        assert!(matches!(err, Err(RegistryError::AlreadyBound(_))));
    }
}
