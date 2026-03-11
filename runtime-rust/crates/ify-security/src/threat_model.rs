//! Threat model for the desktop-to-canvas execution path — Epic O item 1.
//!
//! This module provides typed representations of threats, threat vectors,
//! attack surfaces, mitigations, and residual risk ratings for the
//! infinityOS desktop-to-canvas execution path.  The model is consumed by
//! the security audit tooling and surfaced in the governance dashboard.
//!
//! See `docs/architecture/security-threat-model.md` for the full narrative.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

/// Classifies the type of threat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatCategory {
    /// Spoofing — impersonating another user, agent, or tool.
    Spoofing,
    /// Tampering — modifying code, data, or artifacts in transit or at rest.
    Tampering,
    /// Repudiation — denying that an action occurred.
    Repudiation,
    /// Information disclosure — exposing sensitive data to unauthorized principals.
    InformationDisclosure,
    /// Denial of service — degrading or preventing availability.
    DenialOfService,
    /// Elevation of privilege — gaining capabilities beyond what is granted.
    ElevationOfPrivilege,
}

/// The execution layer where the threat is most relevant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatLayer {
    /// Desktop shell / OS process boundary.
    Desktop,
    /// Canvas UI surface (node graph, snippets).
    Canvas,
    /// Performer runtime (orchestrator, tool runner).
    Runtime,
    /// Kernel FFI boundary (C↔Rust).
    Kernel,
    /// Mesh artifact bus.
    Mesh,
    /// Agent execution environment.
    Agent,
}

/// Qualitative risk level combining likelihood × impact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Low likelihood and/or low impact.
    Low,
    /// Medium likelihood or impact.
    Medium,
    /// High likelihood and significant impact.
    High,
    /// Critical: exploitable with severe consequences.
    Critical,
}

// ---------------------------------------------------------------------------
// Mitigation
// ---------------------------------------------------------------------------

/// A specific control that reduces the likelihood or impact of a threat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mitigation {
    /// Short identifier (e.g. `"sandbox-isolation"`).
    pub id: String,
    /// Human-readable description of the control.
    pub description: String,
    /// Whether this control has been implemented.
    pub implemented: bool,
}

impl Mitigation {
    /// Create a new mitigation entry.
    pub fn new(id: impl Into<String>, description: impl Into<String>, implemented: bool) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            implemented,
        }
    }
}

// ---------------------------------------------------------------------------
// ThreatEntry
// ---------------------------------------------------------------------------

/// A single threat in the desktop-to-canvas threat model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatEntry {
    /// Unique threat identifier (e.g. `"T-01"`).
    pub id: String,
    /// Short title.
    pub title: String,
    /// STRIDE category.
    pub category: ThreatCategory,
    /// Execution layer most at risk.
    pub layer: ThreatLayer,
    /// Description of the attack scenario.
    pub description: String,
    /// Inherent risk before mitigations.
    pub inherent_risk: RiskLevel,
    /// Residual risk after all implemented mitigations.
    pub residual_risk: RiskLevel,
    /// Controls that address this threat.
    pub mitigations: Vec<Mitigation>,
}

impl ThreatEntry {
    /// Create a new threat entry.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        category: ThreatCategory,
        layer: ThreatLayer,
        description: impl Into<String>,
        inherent_risk: RiskLevel,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            category,
            layer,
            description: description.into(),
            inherent_risk,
            residual_risk: inherent_risk,
            mitigations: Vec::new(),
        }
    }

    /// Attach a mitigation to this threat and recompute residual risk.
    pub fn add_mitigation(mut self, m: Mitigation, residual: RiskLevel) -> Self {
        self.mitigations.push(m);
        self.residual_risk = residual;
        self
    }
}

// ---------------------------------------------------------------------------
// ThreatModel
// ---------------------------------------------------------------------------

/// The complete threat model for the desktop-to-canvas execution path.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ThreatModel {
    entries: Vec<ThreatEntry>,
}

impl ThreatModel {
    /// Create an empty threat model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the canonical desktop-to-canvas threat model.
    ///
    /// Returns a pre-populated model covering the key STRIDE threats
    /// identified in `docs/architecture/security-threat-model.md`.
    pub fn desktop_to_canvas() -> Self {
        let mut model = Self::new();

        model.add(
            ThreatEntry::new(
                "T-01",
                "Agent impersonation via forged TaskID",
                ThreatCategory::Spoofing,
                ThreatLayer::Agent,
                "A malicious agent claims a TaskID belonging to a different dimension, \
                 enabling it to read or write artifacts it does not own.",
                RiskLevel::High,
            )
            .add_mitigation(
                Mitigation::new(
                    "identity-verification",
                    "Principal identity verified on every ActionLog emission; \
                     TaskID ownership checked against DimensionId at the orchestrator boundary.",
                    true,
                ),
                RiskLevel::Low,
            ),
        );

        model.add(
            ThreatEntry::new(
                "T-02",
                "Artifact tampering via unsigned mesh write",
                ThreatCategory::Tampering,
                ThreatLayer::Mesh,
                "An attacker intercepts a mesh artifact write and replaces payload bytes, \
                 causing downstream consumers to execute corrupted data.",
                RiskLevel::High,
            )
            .add_mitigation(
                Mitigation::new(
                    "artifact-signing",
                    "All runtime and deploy artifacts are signed; consumers verify the \
                     signature before processing (see artifact_signing module).",
                    true,
                ),
                RiskLevel::Low,
            ),
        );

        model.add(
            ThreatEntry::new(
                "T-03",
                "Privileged action repudiation",
                ThreatCategory::Repudiation,
                ThreatLayer::Runtime,
                "A user or agent denies having triggered a privileged action (deploy, \
                 secret read, admin) because no tamper-evident log exists.",
                RiskLevel::Medium,
            )
            .add_mitigation(
                Mitigation::new(
                    "audit-trail",
                    "All privileged actions emit signed ActionLog entries persisted \
                     to the mesh with hash-chaining (see audit module).",
                    true,
                ),
                RiskLevel::Low,
            ),
        );

        model.add(
            ThreatEntry::new(
                "T-04",
                "Secret leakage through canvas node output",
                ThreatCategory::InformationDisclosure,
                ThreatLayer::Canvas,
                "A snippet or node logs or returns a secret value that is rendered \
                 in the canvas UI or stored unredacted in an artifact.",
                RiskLevel::Critical,
            )
            .add_mitigation(
                Mitigation::new(
                    "secret-redaction",
                    "The Redactor intercepts all string outputs at the canvas boundary \
                     and replaces registered secret patterns with `[REDACTED]`.",
                    true,
                ),
                RiskLevel::Low,
            ),
        );

        model.add(
            ThreatEntry::new(
                "T-05",
                "Sandbox escape via unrestricted filesystem access",
                ThreatCategory::ElevationOfPrivilege,
                ThreatLayer::Runtime,
                "A tool or snippet accesses files outside its declared sandbox path, \
                 potentially reading sensitive host files.",
                RiskLevel::Critical,
            )
            .add_mitigation(
                Mitigation::new(
                    "sandbox-enforcement",
                    "SandboxPolicy enforces allowed path prefixes, network hosts, and \
                     model IDs before any tool invocation (see sandbox module).",
                    true,
                ),
                RiskLevel::Low,
            ),
        );

        model.add(
            ThreatEntry::new(
                "T-06",
                "Capability escalation through unvalidated boundary input",
                ThreatCategory::ElevationOfPrivilege,
                ThreatLayer::Kernel,
                "Malformed input at a C↔Rust FFI boundary triggers undefined behavior \
                 or allows an attacker to craft a payload that bypasses capability checks.",
                RiskLevel::High,
            )
            .add_mitigation(
                Mitigation::new(
                    "boundary-input-validation",
                    "InputValidator runs schema and invariant checks at every layer \
                     boundary before data is passed deeper into the system.",
                    true,
                ),
                RiskLevel::Low,
            ),
        );

        model.add(
            ThreatEntry::new(
                "T-07",
                "Supply chain compromise via unsigned dependency",
                ThreatCategory::Tampering,
                ThreatLayer::Desktop,
                "A compromised transitive dependency introduces malicious code that \
                 executes during build or runtime.",
                RiskLevel::High,
            )
            .add_mitigation(
                Mitigation::new(
                    "sbom-and-signature-verification",
                    "SBOM is generated at build time; component signatures are verified \
                     before installation (see supply_chain module).",
                    true,
                ),
                RiskLevel::Medium,
            ),
        );

        model.add(
            ThreatEntry::new(
                "T-08",
                "Denial of service via unbounded task queue",
                ThreatCategory::DenialOfService,
                ThreatLayer::Runtime,
                "A rogue agent submits an unbounded number of tasks, exhausting \
                 scheduler capacity and starving legitimate workloads.",
                RiskLevel::Medium,
            )
            .add_mitigation(
                Mitigation::new(
                    "policy-engine-rate-limiting",
                    "PolicyEngine enforces per-dimension rate limits and quota rules \
                     before task submission is accepted.",
                    true,
                ),
                RiskLevel::Low,
            ),
        );

        model
    }

    /// Add a threat entry to the model.
    pub fn add(&mut self, entry: ThreatEntry) {
        self.entries.push(entry);
    }

    /// Iterate over all threat entries.
    pub fn entries(&self) -> &[ThreatEntry] {
        &self.entries
    }

    /// Return entries filtered by risk level (residual risk ≥ `min_risk`).
    pub fn entries_at_least(&self, min_risk: RiskLevel) -> Vec<&ThreatEntry> {
        self.entries
            .iter()
            .filter(|e| e.residual_risk >= min_risk)
            .collect()
    }

    /// Total number of threats in the model.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the model contains no threat entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_to_canvas_model_is_populated() {
        let model = ThreatModel::desktop_to_canvas();
        assert!(!model.is_empty(), "model must contain at least one threat");
    }

    #[test]
    fn all_critical_threats_have_mitigations() {
        let model = ThreatModel::desktop_to_canvas();
        for entry in model.entries() {
            if entry.inherent_risk == RiskLevel::Critical {
                assert!(
                    !entry.mitigations.is_empty(),
                    "critical threat '{}' must have at least one mitigation",
                    entry.id
                );
            }
        }
    }

    #[test]
    fn residual_risk_le_inherent_risk() {
        let model = ThreatModel::desktop_to_canvas();
        for entry in model.entries() {
            assert!(
                entry.residual_risk <= entry.inherent_risk,
                "threat '{}': residual risk must not exceed inherent risk",
                entry.id
            );
        }
    }

    #[test]
    fn filter_by_min_risk() {
        let model = ThreatModel::desktop_to_canvas();
        let high_plus = model.entries_at_least(RiskLevel::High);
        // All entries with residual High or Critical
        for e in &high_plus {
            assert!(e.residual_risk >= RiskLevel::High);
        }
    }

    #[test]
    fn mitigation_builder() {
        let m = Mitigation::new("m-1", "test mitigation", true);
        assert!(m.implemented);
        let entry = ThreatEntry::new(
            "T-99",
            "Test threat",
            ThreatCategory::Spoofing,
            ThreatLayer::Canvas,
            "desc",
            RiskLevel::Medium,
        )
        .add_mitigation(m, RiskLevel::Low);
        assert_eq!(entry.residual_risk, RiskLevel::Low);
        assert_eq!(entry.mitigations.len(), 1);
    }
}
