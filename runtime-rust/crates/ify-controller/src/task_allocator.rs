//! Per-dimension monotonic TaskID allocator.
//!
//! Satisfies Epic B requirement:
//! > Implement global-per-dimension TaskID allocator (monotonic + ULID/UUIDv7)
//! > + deterministic derivation option.
//!
//! ## Monotonic allocation
//!
//! Each registered dimension maintains its own allocation slot.  IDs are
//! generated with [`Uuid::now_v7`], which encodes a millisecond-precision
//! timestamp in the high bits, guaranteeing non-decreasing ordering for IDs
//! produced within the same process (UUID v7 §5.2 monotonicity requirement).
//!
//! ## Deterministic derivation
//!
//! [`TaskAllocator::derive`] produces a stable [`TaskId`] from a `(dimension, name)`
//! pair using UUID v5 (SHA-1, RFC 4122 §4.3) with a fixed application namespace.
//! The same inputs always produce the same output, regardless of wall-clock time.

use std::collections::HashMap;
use std::sync::Mutex;

use ify_core::{DimensionId, TaskId};
use thiserror::Error;
use tracing::debug;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the task allocator.
#[derive(Debug, Error)]
pub enum AllocatorError {
    /// The dimension was not registered before `next()` was called.
    #[error("dimension {0} is not registered in this allocator")]
    UnknownDimension(DimensionId),
}

// ---------------------------------------------------------------------------
// TaskAllocator
// ---------------------------------------------------------------------------

/// Fixed UUID v5 application namespace for deterministic TaskId derivation.
///
/// Generated once (offline) to guarantee stability across releases.
/// Namespace: "ify:task-allocator:v1" via UUID v5 of Uuid::NAMESPACE_OID.
const DERIVE_NAMESPACE: Uuid =
    uuid::uuid!("7a9f3c1e-8b4d-5e2f-a601-3c7d9e0b1f24");

/// Per-dimension, monotonically-increasing TaskID allocator.
///
/// ## Usage
///
/// ```rust
/// use ify_controller::task_allocator::TaskAllocator;
/// use ify_core::DimensionId;
///
/// let allocator = TaskAllocator::new();
/// let dim = DimensionId::new();
/// allocator.register_dimension(dim);
///
/// let id1 = allocator.next(dim).unwrap();
/// let id2 = allocator.next(dim).unwrap();
/// assert!(id1 <= id2);
/// ```
#[derive(Debug, Default)]
pub struct TaskAllocator {
    /// Tracks the last-generated TaskId per dimension for auditing.
    last: Mutex<HashMap<DimensionId, TaskId>>,
}

impl TaskAllocator {
    /// Create a new, empty allocator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a dimension so that [`next`][TaskAllocator::next] can be
    /// called for it.  Calling this on an already-registered dimension is a
    /// no-op.
    pub fn register_dimension(&self, dim: DimensionId) {
        let mut map = self.last.lock().expect("allocator lock poisoned");
        map.entry(dim).or_default();
        debug!(dimension = %dim, "dimension registered in task allocator");
    }

    /// Generate the next monotonic [`TaskId`] for `dim`.
    ///
    /// # Errors
    ///
    /// Returns [`AllocatorError::UnknownDimension`] if `dim` has not been
    /// registered via [`register_dimension`][TaskAllocator::register_dimension].
    pub fn next(&self, dim: DimensionId) -> Result<TaskId, AllocatorError> {
        let mut map = self.last.lock().expect("allocator lock poisoned");
        if !map.contains_key(&dim) {
            return Err(AllocatorError::UnknownDimension(dim));
        }
        let id = TaskId::new();
        map.insert(dim, id);
        debug!(dimension = %dim, task_id = %id, "task id allocated");
        Ok(id)
    }

    /// Derive a **deterministic** [`TaskId`] from `(dim, name)`.
    ///
    /// Uses UUID v5 (SHA-1) with [`DERIVE_NAMESPACE`] as the namespace and
    /// `"<dim>:<name>"` as the name bytes.  Identical inputs always produce
    /// the same `TaskId`.
    ///
    /// This is useful for idempotent operations where the caller needs a
    /// stable ID that can be reconstructed without shared state.
    pub fn derive(&self, dim: DimensionId, name: &str) -> TaskId {
        let full_name = format!("{dim}:{name}");
        let derived = Uuid::new_v5(&DERIVE_NAMESPACE, full_name.as_bytes());
        debug!(dimension = %dim, name, task_id = %derived, "deterministic task id derived");
        TaskId::from_uuid(derived)
    }

    /// Return the last allocated TaskId for `dim`, if any.
    pub fn last_for(&self, dim: DimensionId) -> Option<TaskId> {
        self.last
            .lock()
            .expect("allocator lock poisoned")
            .get(&dim)
            .copied()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::DimensionId;

    #[test]
    fn next_requires_registered_dimension() {
        let alloc = TaskAllocator::new();
        let dim = DimensionId::new();
        assert!(matches!(alloc.next(dim), Err(AllocatorError::UnknownDimension(_))));
    }

    #[test]
    fn next_is_monotonic_within_dimension() {
        let alloc = TaskAllocator::new();
        let dim = DimensionId::new();
        alloc.register_dimension(dim);

        let ids: Vec<TaskId> = (0..16).map(|_| alloc.next(dim).unwrap()).collect();
        for w in ids.windows(2) {
            assert!(w[0] <= w[1], "TaskIds must be non-decreasing");
        }
    }

    #[test]
    fn next_different_dimensions_are_independent() {
        let alloc = TaskAllocator::new();
        let d1 = DimensionId::new();
        let d2 = DimensionId::new();
        alloc.register_dimension(d1);
        alloc.register_dimension(d2);

        // IDs from different dimensions are globally unique (UUID v7)
        let id1 = alloc.next(d1).unwrap();
        let id2 = alloc.next(d2).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn derive_is_deterministic() {
        let alloc = TaskAllocator::new();
        let dim = DimensionId::new();

        let a = alloc.derive(dim, "my-task");
        let b = alloc.derive(dim, "my-task");
        assert_eq!(a, b, "same inputs must produce the same TaskId");
    }

    #[test]
    fn derive_differs_by_name() {
        let alloc = TaskAllocator::new();
        let dim = DimensionId::new();

        let a = alloc.derive(dim, "task-a");
        let b = alloc.derive(dim, "task-b");
        assert_ne!(a, b);
    }

    #[test]
    fn derive_differs_by_dimension() {
        let alloc = TaskAllocator::new();
        let d1 = DimensionId::new();
        let d2 = DimensionId::new();

        let a = alloc.derive(d1, "my-task");
        let b = alloc.derive(d2, "my-task");
        assert_ne!(a, b, "same name in different dimensions must differ");
    }

    #[test]
    fn register_dimension_is_idempotent() {
        let alloc = TaskAllocator::new();
        let dim = DimensionId::new();
        alloc.register_dimension(dim);
        alloc.register_dimension(dim); // must not panic
        assert!(alloc.next(dim).is_ok());
    }
}
