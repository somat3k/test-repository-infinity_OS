//! Sandboxed tool execution — Epic O item 6.
//!
//! Provides [`SandboxPolicy`] and [`SandboxEnforcer`] that gate every tool
//! invocation against a declared [`SandboxProfile`].  Each profile lists
//! the filesystem path prefixes, network hosts, and model IDs that the tool
//! is allowed to access.
//!
//! Before any tool is invoked the caller must call
//! [`SandboxEnforcer::check`] with the target resource; if the check fails a
//! [`SandboxError`] is returned and the invocation must be aborted.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ify_controller::action_log::{ActionLog, ActionLogEntry, Actor, EventType};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the sandbox subsystem.
#[derive(Debug, Error)]
pub enum SandboxError {
    /// The requested filesystem path is outside the allowed prefixes.
    #[error("sandbox: path '{path}' not in allowed prefix list for tool '{tool}'")]
    PathNotAllowed {
        /// Tool that attempted the access.
        tool: String,
        /// The denied path.
        path: String,
    },
    /// The tool is not permitted to write to the filesystem.
    #[error("sandbox: filesystem write access denied for tool '{tool}' on path '{path}'")]
    WriteNotAllowed {
        /// Tool that attempted the write.
        tool: String,
        /// The denied path.
        path: String,
    },
    /// The requested network host is not in the allow-list.
    #[error("sandbox: network host '{host}' not allowed for tool '{tool}'")]
    HostNotAllowed {
        /// Tool that attempted the access.
        tool: String,
        /// The denied host.
        host: String,
    },
    /// The requested model ID is not in the allow-list.
    #[error("sandbox: model '{model_id}' not allowed for tool '{tool}'")]
    ModelNotAllowed {
        /// Tool that attempted the access.
        tool: String,
        /// The denied model ID.
        model_id: String,
    },
    /// No sandbox profile is registered for the tool.
    #[error("sandbox: no profile registered for tool '{0}'")]
    NoProfile(String),
}

// ---------------------------------------------------------------------------
// PathAccess
// ---------------------------------------------------------------------------

/// Specifies whether a filesystem path access is a read or a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathAccess {
    /// Read-only access.
    Read,
    /// Write access (requires `allow_fs_write` in the [`SandboxProfile`]).
    Write,
}

// ---------------------------------------------------------------------------
// SandboxProfile
// ---------------------------------------------------------------------------

/// Declares the resource boundaries for a single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxProfile {
    /// Tool identifier (must match the tool name in the registry).
    pub tool_name: String,
    /// Allowed filesystem path prefixes (e.g. `["/tmp/tool-workdir"]`).
    pub allowed_paths: Vec<String>,
    /// Allowed network hosts (e.g. `["api.example.com"]`).
    pub allowed_hosts: Vec<String>,
    /// Allowed model IDs (e.g. `["gpt-4"]`).
    pub allowed_models: Vec<String>,
    /// Whether the tool is permitted to write to the filesystem.
    pub allow_fs_write: bool,
    /// Whether the tool is permitted to make network egress calls.
    pub allow_network: bool,
}

impl SandboxProfile {
    /// Create a maximally restrictive (deny-all) profile for `tool_name`.
    pub fn deny_all(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            allowed_paths: Vec::new(),
            allowed_hosts: Vec::new(),
            allowed_models: Vec::new(),
            allow_fs_write: false,
            allow_network: false,
        }
    }

    /// Allow a filesystem path prefix.
    pub fn with_path(mut self, prefix: impl Into<String>) -> Self {
        self.allowed_paths.push(prefix.into());
        self
    }

    /// Allow a network host.
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.allow_network = true;
        self.allowed_hosts.push(host.into());
        self
    }

    /// Allow an ML model ID.
    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.allowed_models.push(model_id.into());
        self
    }

    /// Enable filesystem write access.
    pub fn with_fs_write(mut self) -> Self {
        self.allow_fs_write = true;
        self
    }
}

// ---------------------------------------------------------------------------
// SandboxResource — the resource being requested
// ---------------------------------------------------------------------------

/// The resource a tool is attempting to access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SandboxResource {
    /// A filesystem path access with the requested operation type.
    Path {
        /// The filesystem path being accessed.
        path: String,
        /// Whether this is a read or write operation.
        access: PathAccess,
    },
    /// A network host (hostname or IP).
    Host(String),
    /// An ML model identifier.
    Model(String),
}

impl SandboxResource {
    /// Convenience constructor for a read access.
    pub fn read_path(path: impl Into<String>) -> Self {
        Self::Path { path: path.into(), access: PathAccess::Read }
    }

    /// Convenience constructor for a write access.
    pub fn write_path(path: impl Into<String>) -> Self {
        Self::Path { path: path.into(), access: PathAccess::Write }
    }
}

// ---------------------------------------------------------------------------
// SandboxPolicy
// ---------------------------------------------------------------------------

/// Stores sandbox profiles indexed by tool name.
#[derive(Debug, Default)]
pub struct SandboxPolicy {
    profiles: HashMap<String, SandboxProfile>,
}

impl SandboxPolicy {
    /// Create an empty policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a profile.  Replaces any existing profile for the same tool.
    pub fn register(&mut self, profile: SandboxProfile) {
        self.profiles.insert(profile.tool_name.clone(), profile);
    }

    /// Look up a profile.
    pub fn get(&self, tool_name: &str) -> Option<&SandboxProfile> {
        self.profiles.get(tool_name)
    }
}

// ---------------------------------------------------------------------------
// SandboxEnforcer
// ---------------------------------------------------------------------------

/// Checks tool resource requests against the registered [`SandboxPolicy`].
///
/// When an [`ActionLog`] is attached via [`SandboxEnforcer::with_action_log`],
/// every denied request emits a [`EventType::SecuritySandboxViolation`] entry
/// in addition to the `tracing::warn!` log.
pub struct SandboxEnforcer<'a> {
    policy: &'a SandboxPolicy,
    action_log: Option<Arc<ActionLog>>,
}

impl<'a> SandboxEnforcer<'a> {
    /// Create an enforcer backed by `policy` with no ActionLog.
    pub fn new(policy: &'a SandboxPolicy) -> Self {
        Self { policy, action_log: None }
    }

    /// Attach an [`ActionLog`] so that violations emit
    /// [`EventType::SecuritySandboxViolation`] entries.
    pub fn with_action_log(mut self, log: Arc<ActionLog>) -> Self {
        self.action_log = Some(log);
        self
    }

    /// Check whether `tool_name` may access `resource`.
    ///
    /// Path checks use [`std::path::Path::starts_with`], which operates at
    /// path-component boundaries, preventing bypass via prefix extensions
    /// (e.g. `/tmp/workdir2/...` does **not** match prefix `/tmp/workdir`).
    ///
    /// Write accesses additionally require `allow_fs_write` to be set in
    /// the tool's [`SandboxProfile`].
    ///
    /// # Errors
    ///
    /// Returns a [`SandboxError`] variant describing the first denied
    /// resource access, or [`SandboxError::NoProfile`] when the tool has no
    /// registered profile.
    pub fn check(
        &self,
        tool_name: &str,
        resource: &SandboxResource,
    ) -> Result<(), SandboxError> {
        let profile = self
            .policy
            .get(tool_name)
            .ok_or_else(|| SandboxError::NoProfile(tool_name.to_owned()))?;

        let result = self.check_inner(tool_name, profile, resource);
        if let Err(ref e) = result {
            self.emit_violation(tool_name, &e.to_string());
        }
        result
    }

    fn check_inner(
        &self,
        tool_name: &str,
        profile: &SandboxProfile,
        resource: &SandboxResource,
    ) -> Result<(), SandboxError> {
        match resource {
            SandboxResource::Path { path, access } => {
                // Use std::path::Path::starts_with for component-boundary checks
                // so that /tmp/workdir2/file does NOT match prefix /tmp/workdir.
                let path_obj = Path::new(path);
                let allowed = profile
                    .allowed_paths
                    .iter()
                    .any(|prefix| path_obj.starts_with(Path::new(prefix)));
                if !allowed {
                    warn!(tool = tool_name, path, "sandbox: path denied");
                    return Err(SandboxError::PathNotAllowed {
                        tool: tool_name.to_owned(),
                        path: path.clone(),
                    });
                }
                // Enforce write restriction.
                if matches!(access, PathAccess::Write) && !profile.allow_fs_write {
                    warn!(tool = tool_name, path, "sandbox: write access denied");
                    return Err(SandboxError::WriteNotAllowed {
                        tool: tool_name.to_owned(),
                        path: path.clone(),
                    });
                }
            }
            SandboxResource::Host(host) => {
                if !profile.allow_network {
                    warn!(tool = tool_name, host, "sandbox: network not allowed");
                    return Err(SandboxError::HostNotAllowed {
                        tool: tool_name.to_owned(),
                        host: host.clone(),
                    });
                }
                let allowed = profile.allowed_hosts.iter().any(|h| h == host);
                if !allowed {
                    warn!(tool = tool_name, host, "sandbox: host denied");
                    return Err(SandboxError::HostNotAllowed {
                        tool: tool_name.to_owned(),
                        host: host.clone(),
                    });
                }
            }
            SandboxResource::Model(model_id) => {
                let allowed = profile.allowed_models.iter().any(|m| m == model_id);
                if !allowed {
                    warn!(tool = tool_name, model_id, "sandbox: model denied");
                    return Err(SandboxError::ModelNotAllowed {
                        tool: tool_name.to_owned(),
                        model_id: model_id.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn emit_violation(&self, tool_name: &str, reason: &str) {
        if let Some(log) = &self.action_log {
            let entry = ActionLogEntry::new(
                EventType::SecuritySandboxViolation,
                Actor::System,
                None,
                None,
                serde_json::json!({"tool": tool_name, "reason": reason}),
            );
            log.append(entry);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_policy() -> SandboxPolicy {
        let mut policy = SandboxPolicy::new();
        policy.register(
            SandboxProfile::deny_all("db-tool")
                .with_path("/tmp/db-workdir")
                .with_host("db.internal")
                .with_model("gpt-4"),
        );
        policy
    }

    #[test]
    fn allowed_path_passes() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(enforcer
            .check("db-tool", &SandboxResource::read_path("/tmp/db-workdir/data.csv"))
            .is_ok());
    }

    #[test]
    fn disallowed_path_fails() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(matches!(
            enforcer.check("db-tool", &SandboxResource::read_path("/etc/passwd")),
            Err(SandboxError::PathNotAllowed { .. })
        ));
    }

    #[test]
    fn path_prefix_extension_bypass_prevented() {
        // /tmp/db-workdir2/file must NOT match the allowed prefix /tmp/db-workdir.
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(matches!(
            enforcer.check("db-tool", &SandboxResource::read_path("/tmp/db-workdir2/evil")),
            Err(SandboxError::PathNotAllowed { .. })
        ));
    }

    #[test]
    fn write_denied_when_flag_not_set() {
        let policy = make_policy(); // allow_fs_write = false
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(matches!(
            enforcer.check("db-tool", &SandboxResource::write_path("/tmp/db-workdir/out.csv")),
            Err(SandboxError::WriteNotAllowed { .. })
        ));
    }

    #[test]
    fn write_allowed_when_flag_is_set() {
        let mut policy = SandboxPolicy::new();
        policy.register(
            SandboxProfile::deny_all("writer-tool")
                .with_path("/tmp/writer-workdir")
                .with_fs_write(),
        );
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(enforcer
            .check("writer-tool", &SandboxResource::write_path("/tmp/writer-workdir/out.csv"))
            .is_ok());
    }

    #[test]
    fn allowed_host_passes() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(enforcer.check("db-tool", &SandboxResource::Host("db.internal".into())).is_ok());
    }

    #[test]
    fn disallowed_host_fails() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(matches!(
            enforcer.check("db-tool", &SandboxResource::Host("evil.example.com".into())),
            Err(SandboxError::HostNotAllowed { .. })
        ));
    }

    #[test]
    fn allowed_model_passes() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(enforcer.check("db-tool", &SandboxResource::Model("gpt-4".into())).is_ok());
    }

    #[test]
    fn disallowed_model_fails() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(matches!(
            enforcer.check("db-tool", &SandboxResource::Model("unknown-model".into())),
            Err(SandboxError::ModelNotAllowed { .. })
        ));
    }

    #[test]
    fn no_profile_fails() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(matches!(
            enforcer.check("unknown-tool", &SandboxResource::read_path("/tmp")),
            Err(SandboxError::NoProfile(_))
        ));
    }

    #[test]
    fn deny_all_blocks_everything() {
        let mut policy = SandboxPolicy::new();
        policy.register(SandboxProfile::deny_all("bare-tool"));
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(enforcer.check("bare-tool", &SandboxResource::read_path("/tmp")).is_err());
        assert!(enforcer.check("bare-tool", &SandboxResource::Host("example.com".into())).is_err());
        assert!(enforcer.check("bare-tool", &SandboxResource::Model("any".into())).is_err());
    }

    #[test]
    fn violation_emits_action_log_event() {
        use ify_controller::action_log::{ActionLog, EventType};
        let log = ActionLog::new(16);
        let mut rx = log.subscribe();
        let mut policy = SandboxPolicy::new();
        policy.register(SandboxProfile::deny_all("restricted"));
        let enforcer = SandboxEnforcer::new(&policy).with_action_log(log);
        let _ = enforcer.check("restricted", &SandboxResource::read_path("/etc/passwd"));

        let entry = rx.try_recv().expect("ActionLog entry must be emitted on violation");
        assert_eq!(entry.event_type, EventType::SecuritySandboxViolation);
    }
}
