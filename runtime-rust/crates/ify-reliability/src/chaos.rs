//! Chaos testing — Epic K item 5.
//!
//! Provides a lightweight chaos-injection framework for exercising fault
//! paths in the replication kernel and orchestrator.
//!
//! # Overview
//!
//! Each [`ChaosScenario`] defines a fault to inject (e.g. task failure,
//! network partition, replica crash) and a [`ChaosPolicy`] that determines
//! when to trigger it.  The [`ChaosEngine`] manages a list of active
//! scenarios, evaluates policies against incoming operation requests, and
//! returns [`ChaosDecision`] values indicating whether to inject a fault.
//!
//! The engine is intentionally deterministic: given the same seed the same
//! sequence of faults will be produced, enabling reproducible chaos tests.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the chaos subsystem.
#[derive(Debug, Error)]
pub enum ChaosError {
    /// A scenario with the given ID already exists.
    #[error("chaos scenario '{0}' already registered")]
    Duplicate(String),
    /// Scenario not found.
    #[error("chaos scenario '{0}' not found")]
    NotFound(String),
    /// The engine lock was poisoned.
    #[error("chaos engine lock poisoned")]
    LockPoisoned,
}

// ---------------------------------------------------------------------------
// FaultKind
// ---------------------------------------------------------------------------

/// The type of fault to inject.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// Force a task to fail with the given error message.
    TaskFailure {
        /// Human-readable reason for the forced failure.
        reason: String,
    },
    /// Delay the operation by the given number of milliseconds.
    LatencyMs {
        /// Delay duration in milliseconds.
        ms: u64,
    },
    /// Crash a replica — simulates kernel-level replica death.
    ReplicaCrash {
        /// Identifier of the replica to crash.
        replica_id: String,
    },
    /// Simulate a partial network partition (drop N % of messages).
    NetworkPartition {
        /// Percentage of messages to drop (0–100).
        drop_percent: u8,
    },
    /// Return a resource-exhausted error.
    ResourceExhausted,
}

// ---------------------------------------------------------------------------
// ChaosPolicy
// ---------------------------------------------------------------------------

/// Controls when the chaos engine injects a fault.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ChaosPolicy {
    /// Inject the fault on every trigger call.
    Always,
    /// Inject the fault once, then automatically disable the scenario.
    Once,
    /// Inject the fault with probability `probability` (0.0–1.0).
    Random {
        /// Probability of injection per evaluation (0.0–1.0).
        probability: f64,
    },
    /// Inject the fault after `after_calls` trigger calls.
    AfterN {
        /// Number of calls that must occur before the fault fires.
        after_calls: u32,
    },
}

// ---------------------------------------------------------------------------
// ChaosScenario
// ---------------------------------------------------------------------------

/// A named chaos scenario consisting of a fault and a trigger policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaosScenario {
    /// Unique identifier for this scenario.
    pub id: String,
    /// Target operation pattern (e.g. `"replication.*"`, `"orchestrator.submit"`).
    pub target: String,
    /// The fault to inject when triggered.
    pub fault: FaultKind,
    /// When to trigger the fault.
    pub policy: ChaosPolicy,
    /// Whether the scenario is currently active.
    pub active: bool,
    /// Number of times this scenario has been triggered.
    pub trigger_count: u32,
}

impl ChaosScenario {
    /// Create a new active scenario.
    pub fn new(
        id: impl Into<String>,
        target: impl Into<String>,
        fault: FaultKind,
        policy: ChaosPolicy,
    ) -> Self {
        Self {
            id: id.into(),
            target: target.into(),
            fault,
            policy,
            active: true,
            trigger_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// ChaosDecision
// ---------------------------------------------------------------------------

/// The outcome of an engine evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChaosDecision {
    /// No fault should be injected; proceed normally.
    Proceed,
    /// Inject the specified fault.
    Inject(FaultKind),
}

impl ChaosDecision {
    /// Returns `true` if a fault will be injected.
    pub fn is_fault(&self) -> bool {
        matches!(self, ChaosDecision::Inject(_))
    }
}

// ---------------------------------------------------------------------------
// ChaosEngine
// ---------------------------------------------------------------------------

/// Manages active chaos scenarios and evaluates them on each call.
///
/// The engine is `Send + Sync`; internal state is protected by a `Mutex`.
pub struct ChaosEngine {
    inner: Mutex<ChaosEngineInner>,
}

struct ChaosEngineInner {
    scenarios: HashMap<String, ChaosScenario>,
    /// Simple deterministic counter used as a pseudo-random seed.
    call_counter: u64,
    /// Seed for reproducible pseudo-random decisions.
    seed: u64,
}

impl ChaosEngine {
    /// Create a new engine with the given deterministic seed.
    pub fn new(seed: u64) -> Self {
        Self {
            inner: Mutex::new(ChaosEngineInner {
                scenarios: HashMap::new(),
                call_counter: 0,
                seed,
            }),
        }
    }

    /// Create a new engine with a zero seed (all random decisions use the
    /// same sequence — useful for reproducible unit tests).
    pub fn deterministic() -> Self {
        Self::new(0)
    }

    /// Register a new chaos scenario.
    pub fn register(&self, scenario: ChaosScenario) -> Result<(), ChaosError> {
        let mut inner = self.inner.lock().map_err(|_| ChaosError::LockPoisoned)?;
        if inner.scenarios.contains_key(&scenario.id) {
            return Err(ChaosError::Duplicate(scenario.id.clone()));
        }
        inner.scenarios.insert(scenario.id.clone(), scenario);
        Ok(())
    }

    /// Deactivate a scenario by ID (does not remove it).
    pub fn deactivate(&self, id: &str) -> Result<(), ChaosError> {
        let mut inner = self.inner.lock().map_err(|_| ChaosError::LockPoisoned)?;
        let scenario = inner
            .scenarios
            .get_mut(id)
            .ok_or_else(|| ChaosError::NotFound(id.to_string()))?;
        scenario.active = false;
        Ok(())
    }

    /// Evaluate the engine for the given `operation` string.
    ///
    /// The engine scans all active scenarios whose `target` is a prefix-match
    /// for `operation` and applies the first matching scenario's policy.
    pub fn evaluate(&self, operation: &str) -> Result<ChaosDecision, ChaosError> {
        let mut inner = self.inner.lock().map_err(|_| ChaosError::LockPoisoned)?;
        inner.call_counter = inner.call_counter.wrapping_add(1);
        let counter = inner.call_counter;
        let seed = inner.seed;

        // Find the first active matching scenario.
        // We collect IDs first to avoid borrowing issues.
        let matching_id: Option<String> = inner
            .scenarios
            .values()
            .filter(|s| s.active && operation.starts_with(&s.target))
            .map(|s| s.id.clone())
            .next();

        let id = match matching_id {
            Some(id) => id,
            None => return Ok(ChaosDecision::Proceed),
        };

        let scenario = inner.scenarios.get_mut(&id).unwrap();
        scenario.trigger_count += 1;
        let trigger_count = scenario.trigger_count;

        let inject = match &scenario.policy {
            ChaosPolicy::Always => true,
            ChaosPolicy::Once => {
                if trigger_count == 1 {
                    scenario.active = false;
                    true
                } else {
                    false
                }
            }
            ChaosPolicy::Random { probability } => {
                // xorshift-inspired hash for a simple, deterministic pseudo-random
                // decision given a seed and call counter. The constant 0x9e37_79b9_7f4a_7c15
                // is the 64-bit golden-ratio fractional expansion used by xxHash/Fibonacci
                // hashing; it produces good avalanche properties for seeding.
                let hash = seed
                    .wrapping_add(counter)
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    ^ (counter >> 17);
                let normalised = (hash & 0xFFFF_FFFF) as f64 / u32::MAX as f64;
                normalised < *probability
            }
            ChaosPolicy::AfterN { after_calls } => trigger_count > *after_calls,
        };

        if inject {
            info!(
                scenario = %scenario.id,
                operation,
                fault = ?scenario.fault,
                "chaos.fault_injected"
            );
            Ok(ChaosDecision::Inject(scenario.fault.clone()))
        } else {
            Ok(ChaosDecision::Proceed)
        }
    }

    /// Return a snapshot of all registered scenarios.
    pub fn snapshot(&self) -> Result<Vec<ChaosScenario>, ChaosError> {
        let inner = self.inner.lock().map_err(|_| ChaosError::LockPoisoned)?;
        Ok(inner.scenarios.values().cloned().collect())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_policy_injects() {
        let engine = ChaosEngine::deterministic();
        engine
            .register(ChaosScenario::new(
                "s1",
                "replication.",
                FaultKind::ReplicaCrash {
                    replica_id: "r1".into(),
                },
                ChaosPolicy::Always,
            ))
            .unwrap();
        let decision = engine.evaluate("replication.spawn").unwrap();
        assert!(decision.is_fault());
    }

    #[test]
    fn once_policy_triggers_once() {
        let engine = ChaosEngine::deterministic();
        engine
            .register(ChaosScenario::new(
                "s1",
                "orchestrator.",
                FaultKind::TaskFailure {
                    reason: "test".into(),
                },
                ChaosPolicy::Once,
            ))
            .unwrap();
        let first = engine.evaluate("orchestrator.submit").unwrap();
        let second = engine.evaluate("orchestrator.submit").unwrap();
        assert!(first.is_fault());
        assert!(!second.is_fault());
    }

    #[test]
    fn no_match_returns_proceed() {
        let engine = ChaosEngine::deterministic();
        let decision = engine.evaluate("mesh.write").unwrap();
        assert!(!decision.is_fault());
    }

    #[test]
    fn duplicate_registration_fails() {
        let engine = ChaosEngine::deterministic();
        let s = ChaosScenario::new("s1", "x.", FaultKind::ResourceExhausted, ChaosPolicy::Always);
        engine.register(s.clone()).unwrap();
        let s2 =
            ChaosScenario::new("s1", "x.", FaultKind::ResourceExhausted, ChaosPolicy::Always);
        assert!(engine.register(s2).is_err());
    }

    #[test]
    fn deactivate_stops_injection() {
        let engine = ChaosEngine::deterministic();
        engine
            .register(ChaosScenario::new(
                "s1",
                "replication.",
                FaultKind::ResourceExhausted,
                ChaosPolicy::Always,
            ))
            .unwrap();
        engine.deactivate("s1").unwrap();
        let decision = engine.evaluate("replication.spawn").unwrap();
        assert!(!decision.is_fault());
    }

    #[test]
    fn after_n_policy_waits() {
        let engine = ChaosEngine::deterministic();
        engine
            .register(ChaosScenario::new(
                "s1",
                "orch.",
                FaultKind::ResourceExhausted,
                ChaosPolicy::AfterN { after_calls: 2 },
            ))
            .unwrap();
        // First two calls should not inject.
        let d1 = engine.evaluate("orch.submit").unwrap();
        let d2 = engine.evaluate("orch.submit").unwrap();
        let d3 = engine.evaluate("orch.submit").unwrap();
        assert!(!d1.is_fault());
        assert!(!d2.is_fault());
        assert!(d3.is_fault());
    }
}
