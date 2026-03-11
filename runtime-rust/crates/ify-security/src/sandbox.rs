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
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum SandboxResource {
    /// A filesystem path.
    Path(String),
    /// A network host (hostname or IP).
    Host(String),
    /// An ML model identifier.
    Model(String),
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
pub struct SandboxEnforcer<'a> {
    policy: &'a SandboxPolicy,
}

impl<'a> SandboxEnforcer<'a> {
    /// Create an enforcer backed by `policy`.
    pub fn new(policy: &'a SandboxPolicy) -> Self {
        Self { policy }
    }

    /// Check whether `tool_name` may access `resource`.
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

        match resource {
            SandboxResource::Path(path) => {
                let allowed = profile
                    .allowed_paths
                    .iter()
                    .any(|prefix| path.starts_with(prefix.as_str()));
                if !allowed {
                    warn!(tool = tool_name, path, "sandbox: path denied");
                    return Err(SandboxError::PathNotAllowed {
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
            .check("db-tool", &SandboxResource::Path("/tmp/db-workdir/data.csv".into()))
            .is_ok());
    }

    #[test]
    fn disallowed_path_fails() {
        let policy = make_policy();
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(matches!(
            enforcer.check("db-tool", &SandboxResource::Path("/etc/passwd".into())),
            Err(SandboxError::PathNotAllowed { .. })
        ));
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
            enforcer.check("unknown-tool", &SandboxResource::Path("/tmp".into())),
            Err(SandboxError::NoProfile(_))
        ));
    }

    #[test]
    fn deny_all_blocks_everything() {
        let mut policy = SandboxPolicy::new();
        policy.register(SandboxProfile::deny_all("bare-tool"));
        let enforcer = SandboxEnforcer::new(&policy);
        assert!(enforcer.check("bare-tool", &SandboxResource::Path("/tmp".into())).is_err());
        assert!(enforcer.check("bare-tool", &SandboxResource::Host("example.com".into())).is_err());
        assert!(enforcer.check("bare-tool", &SandboxResource::Model("any".into())).is_err());
    }
}
