//! # contract — Contract Tests for Interfaces (IDL)
//!
//! Defines the contract testing framework for all cross-layer interfaces in
//! infinityOS.  Each interface (mesh artifact API, event bus, node execution,
//! editor integration) has a [`InterfaceContract`] that specifies the
//! invariants that any conforming implementation must satisfy.
//!
//! Concrete implementations prove conformance by registering a
//! [`ConformanceProbe`] against the contract.  The
//! [`ContractTestRunner`] executes all registered probes and records pass/fail.
//!
//! This module is language-layer agnostic: the C↔Rust FFI ABI contracts are
//! captured here as first-class entries so that the quality gate can track
//! them alongside the Rust trait contracts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the contract testing module.
#[derive(Debug, Error)]
pub enum ContractError {
    /// A contract with the given name already exists.
    #[error("duplicate contract: {0}")]
    DuplicateContract(String),
    /// A referenced contract does not exist.
    #[error("unknown contract: {0}")]
    UnknownContract(String),
}

// ---------------------------------------------------------------------------
// Interface layer
// ---------------------------------------------------------------------------

/// The architectural layer that owns the interface being tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceLayer {
    /// Kernel C API (ABI-stable FFI surface).
    KernelAbi,
    /// Rust crate-level trait surface (`ify-interfaces`).
    RustTraits,
    /// Mesh artifact wire protocol.
    MeshProtocol,
    /// ActionLog event schema.
    ActionLogSchema,
    /// Editor integration API.
    EditorApi,
}

// ---------------------------------------------------------------------------
// Contract invariant
// ---------------------------------------------------------------------------

/// A single invariant that a conforming implementation must satisfy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractInvariant {
    /// Unique invariant ID within a contract.
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Whether this invariant is mandatory (failure is blocking) or advisory.
    pub mandatory: bool,
}

impl ContractInvariant {
    /// Create a mandatory invariant.
    pub fn mandatory(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self { id: id.into(), description: description.into(), mandatory: true }
    }

    /// Create an advisory invariant.
    pub fn advisory(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self { id: id.into(), description: description.into(), mandatory: false }
    }
}

// ---------------------------------------------------------------------------
// Interface contract
// ---------------------------------------------------------------------------

/// A contract definition for a single interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceContract {
    /// Unique contract name (e.g., `"mesh-artifact-api-v1"`).
    pub name: String,
    /// Layer this contract belongs to.
    pub layer: InterfaceLayer,
    /// Current semantic version of the contract.
    pub version: String,
    /// Invariants that implementations must satisfy.
    pub invariants: Vec<ContractInvariant>,
}

impl InterfaceContract {
    /// Create a new contract.
    pub fn new(
        name: impl Into<String>,
        layer: InterfaceLayer,
        version: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            layer,
            version: version.into(),
            invariants: vec![],
        }
    }

    /// Add an invariant.
    pub fn with_invariant(mut self, inv: ContractInvariant) -> Self {
        self.invariants.push(inv);
        self
    }
}

// ---------------------------------------------------------------------------
// Conformance probe result
// ---------------------------------------------------------------------------

/// The result of running a single conformance probe for one invariant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    /// Contract name.
    pub contract: String,
    /// Invariant ID.
    pub invariant_id: String,
    /// Whether the invariant is satisfied.
    pub passed: bool,
    /// Optional diagnostic message.
    pub message: Option<String>,
}

impl ProbeResult {
    /// Passing probe result.
    pub fn pass(contract: impl Into<String>, invariant_id: impl Into<String>) -> Self {
        Self { contract: contract.into(), invariant_id: invariant_id.into(), passed: true, message: None }
    }

    /// Failing probe result with a diagnostic message.
    pub fn fail(
        contract: impl Into<String>,
        invariant_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            contract: contract.into(),
            invariant_id: invariant_id.into(),
            passed: false,
            message: Some(message.into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Contract registry
// ---------------------------------------------------------------------------

/// Registry of all known interface contracts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContractRegistry {
    contracts: HashMap<String, InterfaceContract>,
}

impl ContractRegistry {
    /// Build the canonical infinityOS interface contract registry.
    pub fn canonical() -> Self {
        let mut r = Self::default();

        let _ = r.add(
            InterfaceContract::new("mesh-artifact-api-v1", InterfaceLayer::RustTraits, "1.0.0")
                .with_invariant(ContractInvariant::mandatory(
                    "produce-returns-artifact-id",
                    "produce() must return a non-empty ArtifactId on success",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "consume-returns-latest-immutable",
                    "consume() on an Immutable artifact must always return the same bytes",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "snapshot-is-consistent",
                    "snapshot() must capture a point-in-time view with no partial writes",
                ))
                .with_invariant(ContractInvariant::advisory(
                    "diff-patch-idempotent",
                    "Applying a patch twice must yield the same result as applying it once",
                )),
        );

        let _ = r.add(
            InterfaceContract::new("event-bus-api-v1", InterfaceLayer::RustTraits, "1.0.0")
                .with_invariant(ContractInvariant::mandatory(
                    "emit-records-event",
                    "emit() must persist the event before returning",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "replay-ordered",
                    "replay() must return events in emission order",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "correlation-id-propagated",
                    "All emitted events must carry a non-empty correlation_id",
                )),
        );

        let _ = r.add(
            InterfaceContract::new("node-execution-api-v1", InterfaceLayer::RustTraits, "1.0.0")
                .with_invariant(ContractInvariant::mandatory(
                    "plan-returns-dag",
                    "plan() must return a DAG with at least one node",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "execute-emits-started-completed",
                    "execute() must emit NodeStarted and NodeCompleted (or NodeFailed) events",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "report-includes-task-id",
                    "report() output must include the originating TaskId",
                )),
        );

        let _ = r.add(
            InterfaceContract::new("kernel-abi-v1", InterfaceLayer::KernelAbi, "1.0.0")
                .with_invariant(ContractInvariant::mandatory(
                    "ffi-init-returns-zero-on-success",
                    "ify_kernel_init() must return 0 on success, non-zero on failure",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "task-id-monotonic",
                    "Consecutive ify_task_next() calls must return strictly increasing IDs",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "alloc-returns-aligned-pointer",
                    "ify_alloc() must return a pointer aligned to at least 8 bytes, or NULL on OOM",
                )),
        );

        let _ = r.add(
            InterfaceContract::new("editor-integration-api-v1", InterfaceLayer::EditorApi, "1.0.0")
                .with_invariant(ContractInvariant::mandatory(
                    "attach-interpreter-idempotent",
                    "Attaching the same interpreter twice must be a no-op (not an error)",
                ))
                .with_invariant(ContractInvariant::mandatory(
                    "bind-runtime-returns-handle",
                    "bind_runtime() must return a non-null RuntimeHandle",
                )),
        );

        r
    }

    /// Add a contract.
    ///
    /// # Errors
    /// Returns [`ContractError::DuplicateContract`] if already registered.
    pub fn add(&mut self, contract: InterfaceContract) -> Result<(), ContractError> {
        if self.contracts.contains_key(&contract.name) {
            return Err(ContractError::DuplicateContract(contract.name));
        }
        self.contracts.insert(contract.name.clone(), contract);
        Ok(())
    }

    /// Return all contracts.
    pub fn all(&self) -> impl Iterator<Item = &InterfaceContract> {
        self.contracts.values()
    }

    /// Look up a contract by name.
    pub fn get(&self, name: &str) -> Option<&InterfaceContract> {
        self.contracts.get(name)
    }
}

// ---------------------------------------------------------------------------
// Contract test runner
// ---------------------------------------------------------------------------

/// Runs registered conformance probes and records results.
#[derive(Debug, Default)]
pub struct ContractTestRunner {
    results: Vec<ProbeResult>,
}

impl ContractTestRunner {
    /// Record a probe result.
    pub fn record(&mut self, result: ProbeResult) {
        self.results.push(result);
    }

    /// Return all recorded results.
    pub fn results(&self) -> &[ProbeResult] {
        &self.results
    }

    /// Return `true` iff all mandatory invariants across all contracts passed.
    ///
    /// The `registry` is consulted to determine which invariants are mandatory.
    pub fn all_mandatory_passed(&self, registry: &ContractRegistry) -> bool {
        for result in &self.results {
            if result.passed {
                continue;
            }
            // Find the invariant in the registry.
            if let Some(contract) = registry.get(&result.contract) {
                if let Some(inv) = contract.invariants.iter().find(|i| i.id == result.invariant_id) {
                    if inv.mandatory {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Produce a summary of results.
    pub fn summary(&self) -> ContractRunSummary {
        let total = self.results.len();
        let passed = self.results.iter().filter(|r| r.passed).count();
        ContractRunSummary { total, passed, failed: total - passed }
    }
}

/// Summary of a contract test run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRunSummary {
    /// Total probes run.
    pub total: usize,
    /// Probes that passed.
    pub passed: usize,
    /// Probes that failed.
    pub failed: usize,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_registry_has_five_contracts() {
        let r = ContractRegistry::canonical();
        assert_eq!(r.all().count(), 5);
    }

    #[test]
    fn duplicate_contract_is_rejected() {
        let mut r = ContractRegistry::default();
        let c = InterfaceContract::new("c1", InterfaceLayer::RustTraits, "1.0.0");
        r.add(c.clone()).unwrap();
        assert!(r.add(c).is_err());
    }

    #[test]
    fn runner_records_pass_and_fail() {
        let mut runner = ContractTestRunner::default();
        runner.record(ProbeResult::pass("mesh-artifact-api-v1", "produce-returns-artifact-id"));
        runner.record(ProbeResult::fail(
            "mesh-artifact-api-v1",
            "snapshot-is-consistent",
            "returned inconsistent state",
        ));
        let summary = runner.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
    }

    #[test]
    fn all_passing_means_mandatory_passed() {
        let registry = ContractRegistry::canonical();
        let mut runner = ContractTestRunner::default();
        // Simulate all mandatory probes passing.
        for contract in registry.all() {
            for inv in &contract.invariants {
                runner.record(ProbeResult::pass(&contract.name, &inv.id));
            }
        }
        assert!(runner.all_mandatory_passed(&registry));
    }

    #[test]
    fn failing_mandatory_probe_fails_check() {
        let registry = ContractRegistry::canonical();
        let mut runner = ContractTestRunner::default();
        runner.record(ProbeResult::fail(
            "event-bus-api-v1",
            "emit-records-event",
            "event not persisted",
        ));
        assert!(!runner.all_mandatory_passed(&registry));
    }
}
