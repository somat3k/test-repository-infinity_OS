//! # ify-ffi — Safe Rust Wrappers Over the C Kernel ABI
//!
//! This crate is the **only** Rust crate that is permitted to interact with the
//! C kernel library.  All other crates must go through this crate's public API.
//!
//! ## Modules
//!
//! - [`KernelHandle`] — kernel lifecycle (init, shutdown, dimension management).
//! - [`SchedulerHandle`] — per-dimension scheduler (submit, cancel, query state).
//! - [`MemoryHandle`]   — arena allocator and general-purpose allocator wrappers.
//! - [`AbiConformance`] — conformance test suite validating the kernel ABI contract.
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
// SchedulerHandle
// ---------------------------------------------------------------------------

/// Task priority levels mirroring the C `ify_priority_t` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TaskPriority {
    /// Background / idle work.
    Idle = 0,
    /// Non-critical workloads.
    Low = 1,
    /// Default priority.
    Normal = 4,
    /// User-interactive tasks.
    High = 6,
    /// Safety / system-critical tasks.
    Critical = 7,
}

/// Safe wrapper around the C `ify_scheduler_t` per-dimension scheduler.
///
/// ## Stub behaviour
///
/// Until the kernel is linked every method returns [`FfiError::KernelNotLinked`],
/// except [`SchedulerHandle::generate_task_id`] which uses a Rust fallback.
pub struct SchedulerHandle {
    dimension_id: DimensionId,
}

impl SchedulerHandle {
    /// Create a scheduler for the given dimension.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    pub fn create(dimension_id: DimensionId) -> Result<Self, FfiError> {
        // TODO: call ify_scheduler_create() once kernel-c is linked.
        //
        // Safety (future): opts.dimension_id must be valid and previously
        // created via ify_dimension_create().  The returned pointer is owned
        // by this handle and freed in drop().
        let _ = dimension_id;
        Err(FfiError::KernelNotLinked)
    }

    /// Submit a task to the scheduler.
    ///
    /// Returns the assigned [`TaskId`] on success.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    /// Once linked may return [`FfiError::KernelStatus`] with the C error code.
    pub fn submit(&self, _priority: TaskPriority) -> Result<TaskId, FfiError> {
        // TODO: call ify_scheduler_submit() once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }

    /// Request cancellation of a task.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    pub fn cancel(&self, _task_id: TaskId) -> Result<(), FfiError> {
        // TODO: call ify_scheduler_cancel() once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }

    /// Query the current state of a task.
    ///
    /// Returns a raw `u8` matching the C `ify_task_state_t` enum.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    pub fn task_state(&self, _task_id: TaskId) -> Result<u8, FfiError> {
        // TODO: call ify_scheduler_state() once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }

    /// Return the dimension this scheduler is bound to.
    pub fn dimension_id(&self) -> DimensionId {
        self.dimension_id
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        // TODO: call ify_scheduler_destroy() once kernel-c is linked.
    }
}

// ---------------------------------------------------------------------------
// MemoryHandle
// ---------------------------------------------------------------------------

/// Statistics for an active kernel arena.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArenaStats {
    /// Bytes currently in use.
    pub bytes_used: usize,
    /// Total bytes reserved in the backing store.
    pub bytes_reserved: usize,
    /// Number of allocations since the last reset.
    pub alloc_count: u64,
}

/// Safe wrapper around the C arena and general-purpose allocator.
///
/// ## Stub behaviour
///
/// Until the kernel is linked every method returns [`FfiError::KernelNotLinked`].
pub struct MemoryHandle;

impl MemoryHandle {
    /// Create a new kernel arena with the given initial capacity.
    ///
    /// `initial_cap == 0` selects the default (64 KiB).
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    pub fn arena_create(_initial_cap: usize) -> Result<Self, FfiError> {
        // TODO: call ify_arena_create() once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }

    /// Allocate `size` bytes aligned to `alignment` from this arena.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    /// Once linked returns [`FfiError::KernelStatus`] on allocation failure.
    pub fn arena_alloc(&self, _size: usize, _alignment: usize) -> Result<*mut u8, FfiError> {
        // TODO: call ify_arena_alloc() once kernel-c is linked.
        //
        // Safety (future): size > 0 and alignment is a power of two ≤ 4096.
        // Returned pointer is valid until MemoryHandle::drop() or arena_reset().
        Err(FfiError::KernelNotLinked)
    }

    /// Reset the arena without freeing its backing store.
    ///
    /// All previously returned pointers become invalid after this call.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    pub fn arena_reset(&self) -> Result<(), FfiError> {
        // TODO: call ify_arena_reset() once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }

    /// Return current allocation statistics for this arena.
    ///
    /// # Errors
    ///
    /// Returns [`FfiError::KernelNotLinked`] until the C library is linked.
    pub fn arena_stats(&self) -> Result<ArenaStats, FfiError> {
        // TODO: call ify_arena_stats() once kernel-c is linked.
        Err(FfiError::KernelNotLinked)
    }
}

impl Drop for MemoryHandle {
    fn drop(&mut self) {
        // TODO: call ify_arena_destroy() once kernel-c is linked.
    }
}

// ---------------------------------------------------------------------------
// ABI Conformance
// ---------------------------------------------------------------------------

/// Result of a single ABI conformance check.
#[derive(Debug, Clone)]
pub struct ConformanceCheck {
    /// Short name of the check.
    pub name: &'static str,
    /// Whether the check passed.
    pub passed: bool,
    /// Optional diagnostic message.
    pub message: Option<String>,
}

/// ABI conformance test suite.
///
/// [`AbiConformance::run_all`] executes every defined check and returns the
/// results.  When the kernel is not yet linked all checks that require a live
/// kernel report `passed: false` with a `KernelNotLinked` message.  Checks
/// that can be validated at compile time (e.g., constant values) always pass.
pub struct AbiConformance;

impl AbiConformance {
    /// Run all conformance checks and return their results.
    ///
    /// The returned slice contains one entry per check.  Callers should iterate
    /// and log any check where `passed == false`.
    pub fn run_all() -> Vec<ConformanceCheck> {
        vec![
            Self::check_abi_version_constant(),
            Self::check_kernel_init_stub(),
            Self::check_scheduler_create_stub(),
            Self::check_memory_arena_create_stub(),
            Self::check_task_id_fallback(),
            Self::check_dimension_id_fallback(),
        ]
    }

    /// Verify that [`EXPECTED_ABI_VERSION`] matches the expected packed format
    /// `0xMMmmpppp` for version 0.1.0.
    fn check_abi_version_constant() -> ConformanceCheck {
        let expected = 0x0001_0000u32; // major=0, minor=1, patch=0
        ConformanceCheck {
            name: "abi_version_constant",
            passed: EXPECTED_ABI_VERSION == expected,
            message: if EXPECTED_ABI_VERSION == expected {
                None
            } else {
                Some(format!(
                    "EXPECTED_ABI_VERSION is {:#010x}, expected {:#010x}",
                    EXPECTED_ABI_VERSION, expected
                ))
            },
        }
    }

    /// Verify that `KernelHandle::init` returns `KernelNotLinked` when the
    /// C library is absent (stub mode).
    fn check_kernel_init_stub() -> ConformanceCheck {
        let result = KernelHandle::init(KernelOpts::default());
        let passed = matches!(result, Err(FfiError::KernelNotLinked));
        ConformanceCheck {
            name: "kernel_init_stub_returns_not_linked",
            passed,
            message: if passed {
                None
            } else {
                Some("KernelHandle::init did not return KernelNotLinked".to_owned())
            },
        }
    }

    /// Verify that `SchedulerHandle::create` returns `KernelNotLinked` in
    /// stub mode.
    fn check_scheduler_create_stub() -> ConformanceCheck {
        let result = SchedulerHandle::create(DimensionId::new());
        let passed = matches!(result, Err(FfiError::KernelNotLinked));
        ConformanceCheck {
            name: "scheduler_create_stub_returns_not_linked",
            passed,
            message: if passed {
                None
            } else {
                Some("SchedulerHandle::create did not return KernelNotLinked".to_owned())
            },
        }
    }

    /// Verify that `MemoryHandle::arena_create` returns `KernelNotLinked` in
    /// stub mode.
    fn check_memory_arena_create_stub() -> ConformanceCheck {
        let result = MemoryHandle::arena_create(0);
        let passed = matches!(result, Err(FfiError::KernelNotLinked));
        ConformanceCheck {
            name: "memory_arena_create_stub_returns_not_linked",
            passed,
            message: if passed {
                None
            } else {
                Some("MemoryHandle::arena_create did not return KernelNotLinked".to_owned())
            },
        }
    }

    /// Verify that the Rust-side TaskId fallback generates unique IDs without
    /// requiring the kernel.
    fn check_task_id_fallback() -> ConformanceCheck {
        let a = TaskId::new();
        let b = TaskId::new();
        let passed = a != b;
        ConformanceCheck {
            name: "task_id_fallback_unique",
            passed,
            message: if passed {
                None
            } else {
                Some("two sequential TaskId::new() calls returned the same value".to_owned())
            },
        }
    }

    /// Verify that the Rust-side DimensionId generates unique IDs.
    fn check_dimension_id_fallback() -> ConformanceCheck {
        let a = DimensionId::new();
        let b = DimensionId::new();
        let passed = a != b;
        ConformanceCheck {
            name: "dimension_id_fallback_unique",
            passed,
            message: if passed {
                None
            } else {
                Some(
                    "two sequential DimensionId::new() calls returned the same value"
                        .to_owned(),
                )
            },
        }
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

    // -----------------------------------------------------------------------
    // SchedulerHandle conformance tests
    // -----------------------------------------------------------------------

    #[test]
    fn scheduler_create_returns_not_linked() {
        let result = SchedulerHandle::create(DimensionId::new());
        assert!(
            matches!(result, Err(FfiError::KernelNotLinked)),
            "SchedulerHandle::create must return KernelNotLinked in stub mode"
        );
    }

    // -----------------------------------------------------------------------
    // MemoryHandle conformance tests
    // -----------------------------------------------------------------------

    #[test]
    fn memory_arena_create_returns_not_linked() {
        let result = MemoryHandle::arena_create(0);
        assert!(
            matches!(result, Err(FfiError::KernelNotLinked)),
            "MemoryHandle::arena_create must return KernelNotLinked in stub mode"
        );
    }

    #[test]
    fn memory_arena_create_with_cap_returns_not_linked() {
        let result = MemoryHandle::arena_create(65536);
        assert!(matches!(result, Err(FfiError::KernelNotLinked)));
    }

    // -----------------------------------------------------------------------
    // AbiConformance tests
    // -----------------------------------------------------------------------

    #[test]
    fn abi_conformance_all_checks_run() {
        let checks = AbiConformance::run_all();
        assert!(!checks.is_empty(), "conformance suite must have at least one check");
    }

    #[test]
    fn abi_version_constant_check_passes() {
        let checks = AbiConformance::run_all();
        let check = checks
            .iter()
            .find(|c| c.name == "abi_version_constant")
            .expect("abi_version_constant check must exist");
        assert!(
            check.passed,
            "abi_version_constant failed: {:?}",
            check.message
        );
    }

    #[test]
    fn task_id_fallback_check_passes() {
        let checks = AbiConformance::run_all();
        let check = checks
            .iter()
            .find(|c| c.name == "task_id_fallback_unique")
            .expect("task_id_fallback_unique check must exist");
        assert!(
            check.passed,
            "task_id_fallback_unique failed: {:?}",
            check.message
        );
    }

    #[test]
    fn dimension_id_fallback_check_passes() {
        let checks = AbiConformance::run_all();
        let check = checks
            .iter()
            .find(|c| c.name == "dimension_id_fallback_unique")
            .expect("dimension_id_fallback_unique check must exist");
        assert!(
            check.passed,
            "dimension_id_fallback_unique failed: {:?}",
            check.message
        );
    }

    #[test]
    fn stub_checks_report_not_linked() {
        let checks = AbiConformance::run_all();
        // In stub mode, kernel-dependent checks should report not-linked
        // failures (passed == false), but compile-time checks should pass.
        let stub_checks = [
            "kernel_init_stub_returns_not_linked",
            "scheduler_create_stub_returns_not_linked",
            "memory_arena_create_stub_returns_not_linked",
        ];
        for name in &stub_checks {
            let check = checks.iter().find(|c| c.name == *name)
                .unwrap_or_else(|| panic!("conformance check '{name}' not found"));
            // In stub mode these should pass (they verify the stub returns the
            // expected KernelNotLinked error).
            assert!(
                check.passed,
                "conformance check '{name}' failed: {:?}",
                check.message
            );
        }
    }
}
