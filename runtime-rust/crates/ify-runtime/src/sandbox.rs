//! Sandbox integration — capability-gated execution policies.
//!
//! The [`Sandbox`] type enforces capability checks before allowing a task to
//! proceed, bridging the capability bitmask granted by the C kernel (via
//! `ify-ffi`) into Rust code that is capability-aware.
//!
//! ## Model
//!
//! Each [`SandboxPolicy`] declares:
//! - `required` — capabilities the task **must** have to proceed.
//! - `allowed` — the full set of capabilities the task may use.
//! - `resource_limits` — optional CPU/memory/time budgets.
//!
//! [`Sandbox::enter`] validates the policy against the currently-granted
//! capabilities and returns a [`SandboxGuard`] scoped to the task lifetime.
//! Dropping the guard revokes the active sandbox context.

use ify_core::Capabilities;
use thiserror::Error;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by sandbox operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// The active capability set does not satisfy the policy's `required` set.
    #[error("capability denied: required {required:?}, granted {granted:?}")]
    CapabilityDenied {
        /// Capabilities the policy requires.
        required: Capabilities,
        /// Capabilities that were actually granted.
        granted: Capabilities,
    },

    /// A resource limit was exceeded.
    #[error("resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),

    /// The sandbox policy is internally inconsistent.
    #[error("invalid sandbox policy: {0}")]
    InvalidPolicy(String),
}

// ---------------------------------------------------------------------------
// Resource limits
// ---------------------------------------------------------------------------

/// Optional per-task resource budgets.
#[derive(Debug, Clone, Default)]
pub struct ResourceLimits {
    /// Maximum wall-clock time the task may run (milliseconds). `0` = unlimited.
    pub max_wall_ms: u64,
    /// Maximum heap memory the task may allocate (bytes). `0` = unlimited.
    pub max_heap_bytes: u64,
    /// Maximum number of spawned sub-tasks. `0` = unlimited.
    pub max_subtasks: u32,
}

// ---------------------------------------------------------------------------
// SandboxPolicy
// ---------------------------------------------------------------------------

/// Policy describing what a sandboxed task is allowed to do.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Capabilities that **must** be present before the sandbox may be entered.
    pub required: Capabilities,
    /// Full set of capabilities the task is permitted to exercise.
    pub allowed: Capabilities,
    /// Optional resource budgets.
    pub limits: ResourceLimits,
    /// Human-readable label (for logging / audit).
    pub label: String,
}

impl SandboxPolicy {
    /// Create a minimal policy that only requires and allows the given set.
    pub fn minimal(caps: Capabilities) -> Self {
        Self {
            required: caps,
            allowed: caps,
            limits: ResourceLimits::default(),
            label: "minimal".to_owned(),
        }
    }

    /// Validate internal consistency.
    ///
    /// `required` must be a subset of `allowed`.
    pub fn validate(&self) -> Result<(), SandboxError> {
        if !self.allowed.contains(self.required) {
            return Err(SandboxError::InvalidPolicy(format!(
                "required {required:?} is not a subset of allowed {allowed:?}",
                required = self.required,
                allowed = self.allowed,
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SandboxGuard
// ---------------------------------------------------------------------------

/// An active sandbox context.  Drop to exit the sandbox.
///
/// While a `SandboxGuard` is live it represents an acknowledged enforcement
/// contract between the caller and the sandbox.  In a production system the
/// guard would interact with OS-level primitives (seccomp, namespaces, etc.).
/// In the current implementation it is a pure Rust accounting object that
/// records the active policy and emits tracing events.
#[derive(Debug)]
pub struct SandboxGuard {
    policy: SandboxPolicy,
}

impl SandboxGuard {
    /// Return the policy active for this sandbox session.
    pub fn policy(&self) -> &SandboxPolicy {
        &self.policy
    }

    /// Return the capabilities allowed within this sandbox.
    pub fn allowed_caps(&self) -> Capabilities {
        self.policy.allowed
    }

    /// Assert that the given capability is allowed in this sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::CapabilityDenied`] if the capability is not
    /// in the allowed set.
    pub fn assert_capability(&self, cap: Capabilities) -> Result<(), SandboxError> {
        if self.policy.allowed.contains(cap) {
            Ok(())
        } else {
            Err(SandboxError::CapabilityDenied {
                required: cap,
                granted: self.policy.allowed,
            })
        }
    }
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        debug!(label = %self.policy.label, "sandbox exited");
    }
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

/// Sandbox manager — validates and enters capability-gated execution contexts.
///
/// Create one `Sandbox` per runtime instance, passing in the capability set
/// that was granted by the C kernel (via `ify_ffi::KernelHandle::granted_caps`).
/// When the kernel is not yet linked the caller may pass `Capabilities::all()`
/// in development mode or a specific set for testing.
pub struct Sandbox {
    granted: Capabilities,
}

impl Sandbox {
    /// Create a new `Sandbox` with the given set of kernel-granted capabilities.
    pub fn new(granted: Capabilities) -> Self {
        Self { granted }
    }

    /// Return the kernel-granted capability set.
    pub fn granted_caps(&self) -> Capabilities {
        self.granted
    }

    /// Enter a sandboxed execution context.
    ///
    /// Validates the policy, checks that the `required` capabilities are
    /// present in the kernel-granted set, and returns a [`SandboxGuard`]
    /// for the duration of the task.
    ///
    /// # Errors
    ///
    /// - [`SandboxError::InvalidPolicy`] if the policy is internally inconsistent.
    /// - [`SandboxError::CapabilityDenied`] if required capabilities are not granted.
    #[instrument(skip(self), fields(label = %policy.label))]
    pub fn enter(&self, policy: SandboxPolicy) -> Result<SandboxGuard, SandboxError> {
        policy.validate()?;

        if !self.granted.contains(policy.required) {
            let missing = policy.required & !self.granted;
            warn!(
                missing = ?missing,
                required = ?policy.required,
                granted = ?self.granted,
                "sandbox entry denied: missing capabilities"
            );
            return Err(SandboxError::CapabilityDenied {
                required: policy.required,
                granted: self.granted,
            });
        }

        debug!(label = %policy.label, allowed = ?policy.allowed, "sandbox entered");
        Ok(SandboxGuard { policy })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn full_sandbox() -> Sandbox {
        Sandbox::new(Capabilities::all())
    }

    #[test]
    fn enter_succeeds_when_caps_granted() {
        let sb = full_sandbox();
        let policy = SandboxPolicy::minimal(Capabilities::MEMORY | Capabilities::SCHEDULER);
        let guard = sb.enter(policy).expect("enter must succeed");
        assert!(guard.allowed_caps().contains(Capabilities::MEMORY));
    }

    #[test]
    fn enter_fails_when_caps_not_granted() {
        let sb = Sandbox::new(Capabilities::MEMORY); // no NET
        let policy = SandboxPolicy::minimal(Capabilities::NET);
        let err = sb.enter(policy).unwrap_err();
        assert!(matches!(err, SandboxError::CapabilityDenied { .. }));
    }

    #[test]
    fn invalid_policy_rejected() {
        let sb = full_sandbox();
        let policy = SandboxPolicy {
            required: Capabilities::MEMORY | Capabilities::NET,
            allowed: Capabilities::MEMORY, // NET required but not allowed → invalid
            limits: ResourceLimits::default(),
            label: "bad".to_owned(),
        };
        let err = sb.enter(policy).unwrap_err();
        assert!(matches!(err, SandboxError::InvalidPolicy(_)));
    }

    #[test]
    fn guard_assert_capability_ok() {
        let sb = full_sandbox();
        let policy = SandboxPolicy::minimal(Capabilities::FS);
        let guard = sb.enter(policy).unwrap();
        assert!(guard.assert_capability(Capabilities::FS).is_ok());
    }

    #[test]
    fn guard_assert_capability_denied() {
        let sb = Sandbox::new(Capabilities::MEMORY);
        let policy = SandboxPolicy::minimal(Capabilities::MEMORY);
        let guard = sb.enter(policy).unwrap();
        assert!(matches!(
            guard.assert_capability(Capabilities::NET),
            Err(SandboxError::CapabilityDenied { .. })
        ));
    }

    #[test]
    fn no_caps_sandbox_denies_everything() {
        let sb = Sandbox::new(Capabilities::NONE);
        let policy = SandboxPolicy::minimal(Capabilities::MEMORY);
        assert!(sb.enter(policy).is_err());
    }
}
