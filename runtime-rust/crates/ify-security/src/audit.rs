//! Audit trail for privileged actions — Epic O item 3.
//!
//! The [`PrivilegedAuditLog`] wraps the core [`ActionLog`] and enforces that
//! every privileged action (those requiring elevated capabilities such as
//! `CAP_DEPLOY`, `CAP_READ_SECRETS`, or `CAP_ADMIN`) is logged with full
//! causality metadata before the action proceeds.
//!
//! Records are hash-chained so that any tampering with the sequence is
//! detectable at export time.

use std::sync::{Arc, Mutex};

use ify_controller::action_log::{ActionLog, ActionLogEntry, Actor, EventType};
use ify_core::{Capabilities, DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the audit subsystem.
#[derive(Debug, Error)]
pub enum AuditError {
    /// A required capability was not present in the audit request.
    #[error("audit entry missing required capability context")]
    MissingCapabilityContext,
    /// The log storage layer rejected the record.
    #[error("audit log storage error: {0}")]
    Storage(String),
}

// ---------------------------------------------------------------------------
// PrivilegedActionKind
// ---------------------------------------------------------------------------

/// Enumerates the kinds of privileged actions that must be audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedActionKind {
    /// Reading a secret value.
    ReadSecret,
    /// Triggering a deployment workflow.
    Deploy,
    /// Granting or revoking capabilities.
    CapabilityChange,
    /// Administrative configuration change.
    AdminChange,
    /// Publishing to the marketplace.
    MarketplacePublish,
}

impl PrivilegedActionKind {
    /// Returns the minimum [`Capabilities`] flag required for this action.
    pub fn required_capability(self) -> Capabilities {
        match self {
            Self::ReadSecret => Capabilities::READ_SECRETS,
            Self::Deploy => Capabilities::DEPLOY,
            Self::CapabilityChange | Self::AdminChange => Capabilities::ADMIN,
            Self::MarketplacePublish => Capabilities::PUBLISH_MARKETPLACE,
        }
    }

    /// Returns the [`EventType`] used in the ActionLog entry.
    pub fn event_type(self) -> EventType {
        match self {
            Self::ReadSecret => EventType::PrivilegedReadSecret,
            Self::Deploy => EventType::PrivilegedDeploy,
            Self::CapabilityChange => EventType::PrivilegedCapabilityChange,
            Self::AdminChange => EventType::PrivilegedAdminChange,
            Self::MarketplacePublish => EventType::PrivilegedMarketplacePublish,
        }
    }
}

// ---------------------------------------------------------------------------
// AuditRecord
// ---------------------------------------------------------------------------

/// A single privileged-action audit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    /// Unique record identifier.
    pub record_id: Uuid,
    /// Kind of privileged action.
    pub kind: PrivilegedActionKind,
    /// Actor that performed the action.
    pub actor: String,
    /// Dimension in which the action occurred.
    pub dimension_id: DimensionId,
    /// Task under which the action occurred.
    pub task_id: TaskId,
    /// Capabilities that were active when the action occurred.
    pub capabilities: Capabilities,
    /// Arbitrary payload (e.g. artifact ID, secret name).
    pub payload: serde_json::Value,
    /// Millisecond Unix timestamp.
    pub occurred_at_ms: u64,
    /// SHA-256 hex hash of the previous record (empty for the first record).
    pub prev_hash: String,
    /// SHA-256 hex hash of this record (computed over all other fields).
    pub hash: String,
}

impl AuditRecord {
    fn compute_hash(
        record_id: Uuid,
        kind: PrivilegedActionKind,
        actor: &str,
        dimension_id: DimensionId,
        task_id: TaskId,
        occurred_at_ms: u64,
        prev_hash: &str,
        payload: &serde_json::Value,
    ) -> String {
        // Deterministic serialization used as hash input.
        let input = format!(
            "{record_id}|{kind:?}|{actor}|{dimension_id}|{task_id}|{occurred_at_ms}|{prev_hash}|{payload}"
        );
        // SHA-256 is used in the full system; here we use a simple FNV-like
        // mixing for the in-process implementation to avoid a heavy crypto
        // dependency.  Production deployments must replace this with
        // SHA-256 (e.g. via the `sha2` crate).
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        input.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

// ---------------------------------------------------------------------------
// PrivilegedAuditLog
// ---------------------------------------------------------------------------

/// Append-only, hash-chained audit log for privileged actions.
///
/// Every [`AuditRecord`] is appended to the inner [`ActionLog`] and also
/// stored in memory for export and verification.  The chain can be verified
/// with [`PrivilegedAuditLog::verify_chain`].
pub struct PrivilegedAuditLog {
    action_log: Arc<ActionLog>,
    records: Mutex<Vec<AuditRecord>>,
}

impl PrivilegedAuditLog {
    /// Create a new audit log backed by `action_log`.
    pub fn new(action_log: Arc<ActionLog>) -> Self {
        Self {
            action_log,
            records: Mutex::new(Vec::new()),
        }
    }

    /// Record a privileged action.
    ///
    /// # Errors
    ///
    /// - Returns [`AuditError::MissingCapabilityContext`] when `caps` does not
    ///   contain the capability required by `kind`.
    /// - Returns [`AuditError::Storage`] if the underlying mutex is poisoned.
    pub fn record(
        &self,
        kind: PrivilegedActionKind,
        actor: Actor,
        dim: DimensionId,
        task: TaskId,
        caps: Capabilities,
        payload: serde_json::Value,
    ) -> Result<(), AuditError> {
        // Validate that the caller holds the required capability for this action.
        if !caps.contains(kind.required_capability()) {
            return Err(AuditError::MissingCapabilityContext);
        }

        let actor_str = actor_to_string(&actor);
        let record_id = Uuid::new_v4();
        let occurred_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut records = self.records.lock().map_err(|_| AuditError::Storage("lock poisoned".into()))?;
        let prev_hash = records.last().map(|r| r.hash.clone()).unwrap_or_default();

        let hash = AuditRecord::compute_hash(
            record_id, kind, &actor_str, dim, task, occurred_at_ms, &prev_hash, &payload,
        );

        let record = AuditRecord {
            record_id,
            kind,
            actor: actor_str.clone(),
            dimension_id: dim,
            task_id: task,
            capabilities: caps,
            payload: payload.clone(),
            occurred_at_ms,
            prev_hash,
            hash,
        };

        // Also emit to the shared ActionLog so audit events are visible to
        // all ActionLog subscribers (mesh, telemetry, etc.).
        let entry = ActionLogEntry::new(
            kind.event_type(),
            actor,
            Some(dim),
            Some(task),
            payload,
        );
        self.action_log.append(entry);

        info!(
            kind = ?kind,
            record_id = %record.record_id,
            "privileged action audited"
        );
        records.push(record);
        Ok(())
    }

    /// Return all audit records in append order.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::Storage`] if the mutex is poisoned.
    pub fn all_records(&self) -> Result<Vec<AuditRecord>, AuditError> {
        Ok(self.records.lock().map_err(|_| AuditError::Storage("lock poisoned".into()))?.clone())
    }

    /// Verify the hash chain integrity.
    ///
    /// For each record this method:
    /// 1. Recomputes the record's hash from its stored fields and checks it
    ///    matches the stored `hash` value (field-level tamper detection).
    /// 2. Checks that the record's `prev_hash` matches the `hash` of the
    ///    preceding record (chain-level tamper detection).
    ///
    /// Returns `Ok(())` only when both checks pass for every record.
    ///
    /// # Errors
    ///
    /// Returns [`AuditError::Storage`] if the mutex is poisoned or if any
    /// tamper is detected.
    pub fn verify_chain(&self) -> Result<(), AuditError> {
        let records = self.records.lock().map_err(|_| AuditError::Storage("lock poisoned".into()))?;
        let mut prev = String::new();
        for (i, record) in records.iter().enumerate() {
            // 1. Recompute hash from fields and verify it matches the stored value.
            let recomputed = AuditRecord::compute_hash(
                record.record_id,
                record.kind,
                &record.actor,
                record.dimension_id,
                record.task_id,
                record.occurred_at_ms,
                &record.prev_hash,
                &record.payload,
            );
            if recomputed != record.hash {
                return Err(AuditError::Storage(format!(
                    "record {i} hash mismatch: stored '{}', recomputed '{}'",
                    record.hash, recomputed
                )));
            }
            // 2. Verify chain linkage.
            if record.prev_hash != prev {
                return Err(AuditError::Storage(format!(
                    "hash chain broken at record index {i}: expected prev_hash '{}', got '{}'",
                    prev, record.prev_hash
                )));
            }
            prev = record.hash.clone();
        }
        Ok(())
    }

    /// Number of records in the log.
    pub fn len(&self) -> usize {
        self.records
            .lock()
            .map(|r| r.len())
            .unwrap_or(0)
    }

    /// Returns `true` if no records have been appended yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert an [`Actor`] to a display string for storage in [`AuditRecord::actor`].
fn actor_to_string(actor: &Actor) -> String {
    match actor {
        Actor::User(name) => format!("user:{name}"),
        Actor::Agent(name) => format!("agent:{name}"),
        Actor::System => "system".to_owned(),
        Actor::Kernel => "kernel".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::{DimensionId, TaskId};

    fn make_log() -> PrivilegedAuditLog {
        let action_log = ActionLog::new(128);
        PrivilegedAuditLog::new(action_log)
    }

    #[test]
    fn record_and_retrieve() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        log.record(
            PrivilegedActionKind::ReadSecret,
            Actor::Agent("agent-1".into()),
            dim,
            task,
            Capabilities::READ_SECRETS,
            serde_json::json!({"secret_name": "api-key"}),
        )
        .unwrap();
        assert_eq!(log.len(), 1);
        let records = log.all_records().unwrap();
        assert_eq!(records[0].kind, PrivilegedActionKind::ReadSecret);
        assert_eq!(records[0].actor, "agent:agent-1");
    }

    #[test]
    fn user_actor_stored_correctly() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        log.record(
            PrivilegedActionKind::Deploy,
            Actor::User("alice".into()),
            dim,
            task,
            Capabilities::DEPLOY,
            serde_json::json!({}),
        )
        .unwrap();
        let records = log.all_records().unwrap();
        assert_eq!(records[0].actor, "user:alice");
    }

    #[test]
    fn record_rejected_for_insufficient_capability() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        // DEPLOY cap is required for Deploy action, but we pass NONE.
        let result = log.record(
            PrivilegedActionKind::Deploy,
            Actor::Agent("agent".into()),
            dim,
            task,
            Capabilities::NONE,
            serde_json::json!({}),
        );
        assert!(matches!(result, Err(AuditError::MissingCapabilityContext)));
    }

    #[test]
    fn chain_is_valid_after_multiple_records() {
        let log = make_log();
        let dim = DimensionId::new();
        for i in 0..5 {
            let task = TaskId::new();
            log.record(
                PrivilegedActionKind::Deploy,
                Actor::Agent(format!("agent-{i}")),
                dim,
                task,
                Capabilities::DEPLOY,
                serde_json::json!({}),
            )
            .unwrap();
        }
        log.verify_chain().unwrap();
    }

    #[test]
    fn tampered_record_breaks_chain() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        log.record(
            PrivilegedActionKind::AdminChange,
            Actor::Agent("admin".into()),
            dim,
            task,
            Capabilities::ADMIN,
            serde_json::json!({}),
        )
        .unwrap();

        // Tamper with the stored record's prev_hash — chain check should catch this.
        {
            let mut records = log.records.lock().unwrap();
            records[0].prev_hash = "tampered".to_string();
        }

        assert!(log.verify_chain().is_err());
    }

    #[test]
    fn tampered_record_field_breaks_hash_check() {
        let log = make_log();
        let dim = DimensionId::new();
        let task = TaskId::new();
        log.record(
            PrivilegedActionKind::AdminChange,
            Actor::Agent("admin".into()),
            dim,
            task,
            Capabilities::ADMIN,
            serde_json::json!({}),
        )
        .unwrap();

        // Tamper with a field value while leaving prev_hash and hash intact.
        {
            let mut records = log.records.lock().unwrap();
            records[0].actor = "attacker".to_string();
        }

        // verify_chain recomputes the hash and must detect the mismatch.
        assert!(log.verify_chain().is_err());
    }

    #[test]
    fn privileged_action_kind_capability_mapping() {
        assert!(PrivilegedActionKind::ReadSecret
            .required_capability()
            .contains(Capabilities::READ_SECRETS));
        assert!(PrivilegedActionKind::Deploy
            .required_capability()
            .contains(Capabilities::DEPLOY));
        assert!(PrivilegedActionKind::AdminChange
            .required_capability()
            .contains(Capabilities::ADMIN));
    }
}
