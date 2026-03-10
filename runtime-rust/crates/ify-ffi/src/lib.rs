//! # ify-ffi — Safe Rust Wrappers Over the C Kernel ABI
//!
//! This crate is the **only** Rust crate that is permitted to interact with the
//! C kernel library.  All other crates must go through this crate's public API.
//!
//! ## Safety Contract
//!
//! All `unsafe` blocks in this crate must:
//!
//! 1. Reference the specific C function or type they wrap.
//! 2. Document the invariants that make the call safe.
//! 3. Map every error code to a typed [`FfiError`] before surfacing it.
//!
//! ## Status
//!
//! The kernel C library is not yet linked.  This crate exposes the full type
//! surface and stub implementations so that the rest of the workspace can be
//! compiled and tested independently.  Stubs return [`FfiError::KernelNotLinked`]
//! until the CMake build is in place and the link step is enabled in
//! `Cargo.toml`.

#![warn(missing_docs, clippy::all)]

use ify_core::{Capabilities, DimensionId, TaskId};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by FFI boundary calls.
#[derive(Debug, Error)]
pub enum FfiError {
    /// The C kernel library is not yet linked (build step pending).
    #[error("C kernel library not linked — build kernel-c and enable the link step in ify-ffi/Cargo.toml")]
    KernelNotLinked,

    /// The kernel was not initialized before an FFI call was made.
    #[error("kernel not initialized — call KernelHandle::init() first")]
    NotInitialized,

    /// An invalid argument was passed to a kernel function.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// The kernel returned an unexpected status code.
    #[error("kernel error (status={0})")]
    KernelStatus(i32),

    /// ABI version mismatch between this crate and the linked kernel.
    #[error("ABI version mismatch: expected {expected:#010x}, got {got:#010x}")]
    AbiVersionMismatch {
        /// Version this crate was compiled against.
        expected: u32,
        /// Version reported by the linked kernel.
        got: u32,
    },
}

// ---------------------------------------------------------------------------
// ABI version
// ---------------------------------------------------------------------------

/// ABI version this crate was compiled against.
///
/// Must match `INFINITY_KERNEL_VERSION` from `kernel.h`.
pub const EXPECTED_ABI_VERSION: u32 = 0x0001_0000; // 0.1.0

// ---------------------------------------------------------------------------
// KernelHandle
// ---------------------------------------------------------------------------

/// Capabilities requested from the kernel at initialization.
#[derive(Debug, Clone)]
pub struct KernelOpts {
    /// Capabilities to request; kernel may grant a subset.
    pub requested_caps: Capabilities,
    /// Maximum number of concurrent dimensions; 0 for default.
    pub max_dimensions: u32,
}

impl Default for KernelOpts {
    fn default() -> Self {
        Self {
            requested_caps: Capabilities::MEMORY | Capabilities::SCHEDULER,
            max_dimensions: 0,
        }
    }
}

/// Handle representing an initialized kernel session.
///
/// Drop this handle to trigger an orderly kernel shutdown.
pub struct KernelHandle {
    granted_caps: Capabilities,
}

impl KernelHandle {
    /// Initialize the kernel with the provided options.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the CMake link step is in
    /// place.  Once linked, returns [`FfiError::AbiVersionMismatch`] if the
    /// runtime ABI version differs from [`EXPECTED_ABI_VERSION`].
    pub fn init(_opts: KernelOpts) -> Result<Self, FfiError> {
        // TODO: replace stub with real FFI call once kernel-c is linked.
        //
        // Safety (future): ify_kernel_init() must only be called once; the
        // KernelHandle's uniqueness guarantees this via Rust ownership.
        Err(FfiError::KernelNotLinked)
    }

    /// Return the capabilities that were granted at initialization.
    pub fn granted_caps(&self) -> Capabilities {
        self.granted_caps
    }

    /// Create a new isolated execution dimension.
    ///
    /// # Errors
    ///
    /// Returns an error if the kernel is not initialized or the dimension
    /// limit has been reached.
    pub fn create_dimension(&self) -> Result<DimensionId, FfiError> {
        // TODO: replace stub with real FFI call once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }

    /// Destroy a dimension and release all associated kernel resources.
    ///
    /// All schedulers and arenas owned by this dimension must be destroyed
    /// before calling this method.
    pub fn destroy_dimension(&self, _id: DimensionId) -> Result<(), FfiError> {
        // TODO: replace stub with real FFI call once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }

    /// Generate a new globally-unique `TaskId` for the given dimension.
    ///
    /// The returned ID satisfies the invariants in
    /// `docs/architecture/taskid-invariants.md`.
    pub fn generate_task_id(&self, _dimension: DimensionId) -> Result<TaskId, FfiError> {
        // TODO: replace stub with real FFI call once kernel-c is linked.
        // Fallback: generate in Rust until kernel is linked.
        Ok(TaskId::new())
    }
}

impl Drop for KernelHandle {
    fn drop(&mut self) {
        // TODO: call ify_kernel_shutdown() once kernel-c is linked.
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_returns_not_linked() {
        let result = KernelHandle::init(KernelOpts::default());
        assert!(
            matches!(result, Err(FfiError::KernelNotLinked)),
            "stub must return KernelNotLinked until kernel-c is built and linked"
        );
    }

    #[test]
    fn generate_task_id_fallback_works() {
        // The fallback Rust implementation should work even without the kernel.
        let a = TaskId::new();
        let b = TaskId::new();
        assert_ne!(a, b);
    }
}
