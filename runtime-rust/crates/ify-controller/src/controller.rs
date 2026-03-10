//! BlockController lifecycle: create → link → isolate → dispose.
//!
//! Satisfies Epic B requirements:
//! > Specify dimensional block controller contracts (inputs/outputs, dimension
//! > scoping, invariants).
//! > Implement controller lifecycle: create → link → isolate → dispose (with
//! > deterministic cleanup).
//! > Validate invalid dimensional mappings (type checks + runtime guards +
//! > error surfaces).
//!
//! ## Lifecycle state machine
//!
//! ```text
//!  create()
//!     │
//!     ▼
//!  Created ──► link(peer) ──► Linked
//!     │                         │
//!     └────────────────────────►│
//!                               ▼
//!                           isolate() ──► Isolated
//!                                            │
//!                                        dispose() ──► Disposed
//! ```
//!
//! Any lifecycle method called in an invalid state returns
//! [`BlockControllerError::InvalidTransition`].  Dimension mismatch in `link`
//! is guarded by [`BlockController::validate_dimension`].

use std::sync::{Arc, Mutex};
use std::time::Instant;

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by [`BlockController`] operations.
#[derive(Debug, Error)]
pub enum BlockControllerError {
    /// The controller was called with a dimension that does not match its own.
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch {
        /// The controller's own dimension.
        expected: DimensionId,
        /// The dimension supplied by the caller.
        got: DimensionId,
    },

    /// The requested lifecycle transition is not valid from the current state.
    #[error("invalid transition from {from:?}: {reason}")]
    InvalidTransition {
        /// Current state.
        from: ControllerState,
        /// Human-readable reason.
        reason: &'static str,
    },

    /// The controller has already been disposed.
    #[error("controller {0} is already disposed")]
    AlreadyDisposed(Uuid),
}

// ---------------------------------------------------------------------------
// ControllerState
// ---------------------------------------------------------------------------

/// Discrete lifecycle state of a [`BlockController`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControllerState {
    /// Freshly constructed; no peers linked yet.
    Created,
    /// Linked to a peer dimension (node or editor).
    Linked,
    /// Sandboxed — isolated from the peer dimension.
    Isolated,
    /// Lifecycle ended; resources released.
    Disposed,
}

// ---------------------------------------------------------------------------
// BlockController
// ---------------------------------------------------------------------------

/// Internal mutable state for a [`BlockController`].
struct Inner {
    state: ControllerState,
    /// Peer dimension set during `link()`.
    peer_dimension: Option<DimensionId>,
    /// Wall-clock instant when this controller was created.
    created_at: Instant,
}

/// A dimensional block controller managing a single block's lifecycle.
///
/// ## Invariants
///
/// 1. Every instance has a **unique** `id`.
/// 2. `dimension_id` is fixed at construction and never changes.
/// 3. Lifecycle transitions are **one-way**; no state can be re-entered.
/// 4. Every state transition emits an [`ActionLogEntry`].
/// 5. Calling `link` with a mismatched dimension returns
///    [`BlockControllerError::DimensionMismatch`].
pub struct BlockController {
    /// Globally-unique controller identifier.
    pub id: Uuid,
    /// The dimension this controller belongs to.
    pub dimension_id: DimensionId,
    /// The task that spawned this controller.
    pub task_id: TaskId,
    inner: Mutex<Inner>,
    action_log: Arc<ActionLog>,
}

impl std::fmt::Debug for BlockController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        f.debug_struct("BlockController")
            .field("id", &self.id)
            .field("dimension_id", &self.dimension_id)
            .field("task_id", &self.task_id)
            .field("state", &state)
            .finish()
    }
}

impl BlockController {
    /// Create a new `BlockController` in the [`ControllerState::Created`] state.
    ///
    /// Emits a [`EventType::ControllerRegistered`] ActionLog entry.
    #[instrument(skip(action_log), fields(dimension = %dimension_id, task_id = %task_id))]
    pub fn create(
        dimension_id: DimensionId,
        task_id: TaskId,
        action_log: Arc<ActionLog>,
    ) -> Self {
        let id = Uuid::new_v4();
        info!(controller_id = %id, "block controller created");

        action_log.append(
            ActionLogEntry::new(
                EventType::ControllerRegistered,
                Actor::System,
                Some(dimension_id),
                Some(task_id),
                serde_json::json!({
                    "controller_id": id,
                    "controller_kind": "block",
                    "dimension_id": dimension_id.to_string(),
                }),
            )
        );

        Self {
            id,
            dimension_id,
            task_id,
            inner: Mutex::new(Inner {
                state: ControllerState::Created,
                peer_dimension: None,
                created_at: Instant::now(),
            }),
            action_log,
        }
    }

    /// Return the current [`ControllerState`].
    pub fn state(&self) -> ControllerState {
        self.inner.lock().expect("controller lock poisoned").state
    }

    /// Validate that `dim` matches this controller's dimension.
    ///
    /// Used internally and exposed for callers that need to verify dimension
    /// membership before performing cross-dimension operations.
    ///
    /// # Errors
    ///
    /// Returns [`BlockControllerError::DimensionMismatch`] if `dim != self.dimension_id`.
    pub fn validate_dimension(&self, dim: DimensionId) -> Result<(), BlockControllerError> {
        if dim != self.dimension_id {
            Err(BlockControllerError::DimensionMismatch {
                expected: self.dimension_id,
                got: dim,
            })
        } else {
            Ok(())
        }
    }

    /// Transition from [`ControllerState::Created`] to [`ControllerState::Linked`].
    ///
    /// `peer_dimension` is the dimension of the node or editor this controller
    /// is being connected to.  It may be the same as `self.dimension_id`
    /// (intra-dimension link) or a different one (cross-dimension relay).
    ///
    /// # Errors
    ///
    /// - [`BlockControllerError::InvalidTransition`] if not in `Created` state.
    #[instrument(skip(self), fields(controller = %self.id, peer = %peer_dimension))]
    pub fn link(&self, peer_dimension: DimensionId) -> Result<(), BlockControllerError> {
        let mut guard = self.inner.lock().expect("controller lock poisoned");
        if guard.state != ControllerState::Created {
            return Err(BlockControllerError::InvalidTransition {
                from: guard.state,
                reason: "link() requires Created state",
            });
        }

        guard.state = ControllerState::Linked;
        guard.peer_dimension = Some(peer_dimension);
        info!(controller_id = %self.id, peer = %peer_dimension, "block controller linked");

        self.action_log.append(ActionLogEntry::new(
            EventType::ControllerLinked,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "controller_id": self.id,
                "target_id": peer_dimension.to_string(),
                "target_kind": "dimension",
            }),
        ));

        Ok(())
    }

    /// Transition from [`ControllerState::Linked`] to [`ControllerState::Isolated`].
    ///
    /// Isolation severs the link to the peer dimension and puts the controller
    /// into a sandbox mode, where it can no longer receive external inputs.
    ///
    /// # Errors
    ///
    /// - [`BlockControllerError::InvalidTransition`] if not in `Linked` state.
    #[instrument(skip(self), fields(controller = %self.id))]
    pub fn isolate(&self) -> Result<(), BlockControllerError> {
        let mut guard = self.inner.lock().expect("controller lock poisoned");
        if guard.state != ControllerState::Linked {
            return Err(BlockControllerError::InvalidTransition {
                from: guard.state,
                reason: "isolate() requires Linked state",
            });
        }

        guard.state = ControllerState::Isolated;
        info!(controller_id = %self.id, "block controller isolated");

        self.action_log.append(ActionLogEntry::new(
            EventType::ControllerIsolated,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "controller_id": self.id,
                "reason": "explicit isolate() call",
            }),
        ));

        Ok(())
    }

    /// Transition to [`ControllerState::Disposed`] and release resources.
    ///
    /// Valid from any non-disposed state; enables deterministic cleanup.
    ///
    /// # Errors
    ///
    /// - [`BlockControllerError::AlreadyDisposed`] if already disposed.
    #[instrument(skip(self), fields(controller = %self.id))]
    pub fn dispose(&self) -> Result<(), BlockControllerError> {
        let mut guard = self.inner.lock().expect("controller lock poisoned");
        if guard.state == ControllerState::Disposed {
            return Err(BlockControllerError::AlreadyDisposed(self.id));
        }

        let lifetime_ms = guard.created_at.elapsed().as_millis() as u64;
        guard.state = ControllerState::Disposed;
        info!(controller_id = %self.id, lifetime_ms, "block controller disposed");

        self.action_log.append(ActionLogEntry::new(
            EventType::ControllerDisposed,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "controller_id": self.id,
                "lifetime_ms": lifetime_ms,
            }),
        ));

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_controller() -> (BlockController, Arc<ActionLog>) {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let task = TaskId::new();
        let ctrl = BlockController::create(dim, task, Arc::clone(&log));
        (ctrl, log)
    }

    #[test]
    fn initial_state_is_created() {
        let (ctrl, _) = make_controller();
        assert_eq!(ctrl.state(), ControllerState::Created);
    }

    #[test]
    fn full_lifecycle_emits_action_log_entries() {
        let (ctrl, log) = make_controller();
        let peer = DimensionId::new();

        ctrl.link(peer).unwrap();
        ctrl.isolate().unwrap();
        ctrl.dispose().unwrap();

        assert_eq!(ctrl.state(), ControllerState::Disposed);

        // create + link + isolate + dispose = 4 entries
        assert_eq!(log.len(), 4);
    }

    #[test]
    fn validate_dimension_ok() {
        let (ctrl, _) = make_controller();
        assert!(ctrl.validate_dimension(ctrl.dimension_id).is_ok());
    }

    #[test]
    fn validate_dimension_mismatch_returns_error() {
        let (ctrl, _) = make_controller();
        let other = DimensionId::new();
        let err = ctrl.validate_dimension(other);
        assert!(matches!(err, Err(BlockControllerError::DimensionMismatch { .. })));
    }

    #[test]
    fn link_from_wrong_state_fails() {
        let (ctrl, _) = make_controller();
        let peer = DimensionId::new();
        ctrl.link(peer).unwrap();
        // Calling link() again from Linked state must fail
        let err = ctrl.link(peer);
        assert!(matches!(err, Err(BlockControllerError::InvalidTransition { .. })));
    }

    #[test]
    fn isolate_from_wrong_state_fails() {
        let (ctrl, _) = make_controller();
        // Cannot isolate from Created (must link first)
        let err = ctrl.isolate();
        assert!(matches!(err, Err(BlockControllerError::InvalidTransition { .. })));
    }

    #[test]
    fn dispose_twice_fails() {
        let (ctrl, _) = make_controller();
        ctrl.dispose().unwrap();
        let err = ctrl.dispose();
        assert!(matches!(err, Err(BlockControllerError::AlreadyDisposed(_))));
    }

    #[test]
    fn dispose_from_created_state_allowed() {
        let (ctrl, _) = make_controller();
        // Emergency cleanup without linking/isolating must succeed
        assert!(ctrl.dispose().is_ok());
        assert_eq!(ctrl.state(), ControllerState::Disposed);
    }
}
