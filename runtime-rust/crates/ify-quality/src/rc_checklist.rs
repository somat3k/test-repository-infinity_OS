//! # rc_checklist — Release Candidate Validation Checklist
//!
//! Defines the structured release candidate (RC) validation checklist that
//! must be completed before any kernel, runtime, or cross-layer release is
//! tagged.  Each checklist item has a category, evidence requirements, and a
//! mutable completion state.
//!
//! The checklist is designed to be:
//! - **Persisted** as a JSON artifact on the mesh (one per RC tag).
//! - **Exported** as a human-readable Markdown report.
//! - **Queried** by quality gate automation to block invalid releases.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors from the RC checklist module.
#[derive(Debug, Error)]
pub enum RcChecklistError {
    /// An item with the given ID already exists.
    #[error("duplicate checklist item: {0}")]
    DuplicateItem(String),
    /// A referenced item does not exist.
    #[error("unknown checklist item: {0}")]
    UnknownItem(String),
    /// Attempted to sign off on an item that is not yet complete.
    #[error("cannot sign off item '{0}': not yet marked complete")]
    NotComplete(String),
}

// ---------------------------------------------------------------------------
// Item category
// ---------------------------------------------------------------------------

/// Category of a release candidate checklist item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RcCategory {
    /// All required test suites passed.
    Testing,
    /// Performance budgets verified.
    Performance,
    /// Security review completed.
    Security,
    /// Documentation updated.
    Documentation,
    /// Interface/ABI compatibility confirmed.
    Compatibility,
    /// Operational runbook reviewed.
    Operations,
    /// Legal/licensing checks passed.
    Legal,
}

// ---------------------------------------------------------------------------
// Completion state
// ---------------------------------------------------------------------------

/// The completion state of a checklist item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    /// Not yet started.
    Pending,
    /// Marked complete but not yet signed off by a reviewer.
    Complete,
    /// Signed off by a named reviewer.
    SignedOff {
        /// Reviewer name or actor identifier.
        reviewer: String,
        /// ISO 8601 sign-off timestamp.
        timestamp: String,
    },
    /// Explicitly waived (with documented reason).
    Waived {
        /// Actor that waived the item.
        waived_by: String,
        /// Reason for waiver.
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// Checklist item
// ---------------------------------------------------------------------------

/// A single RC checklist item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RcChecklistItem {
    /// Unique item ID.
    pub id: String,
    /// Category.
    pub category: RcCategory,
    /// Human-readable description of what must be done.
    pub description: String,
    /// What evidence satisfies this item (e.g., CI link, artifact ID).
    pub evidence_required: String,
    /// Whether this item is blocking (release cannot proceed without it).
    pub blocking: bool,
    /// Current state.
    pub state: ItemState,
    /// Link to the evidence once provided.
    pub evidence_link: Option<String>,
}

impl RcChecklistItem {
    /// Create a new blocking item in `Pending` state.
    pub fn blocking(
        id: impl Into<String>,
        category: RcCategory,
        description: impl Into<String>,
        evidence_required: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            category,
            description: description.into(),
            evidence_required: evidence_required.into(),
            blocking: true,
            state: ItemState::Pending,
            evidence_link: None,
        }
    }

    /// Create a non-blocking (advisory) item in `Pending` state.
    pub fn advisory(
        id: impl Into<String>,
        category: RcCategory,
        description: impl Into<String>,
        evidence_required: impl Into<String>,
    ) -> Self {
        Self {
            blocking: false,
            ..Self::blocking(id, category, description, evidence_required)
        }
    }

    /// Mark the item complete and attach evidence.
    pub fn mark_complete(&mut self, evidence_link: impl Into<String>) {
        self.state = ItemState::Complete;
        self.evidence_link = Some(evidence_link.into());
    }

    /// Sign off on a complete item.
    ///
    /// # Errors
    /// Returns [`RcChecklistError::NotComplete`] if the item is still `Pending`.
    pub fn sign_off(
        &mut self,
        reviewer: impl Into<String>,
        timestamp: impl Into<String>,
    ) -> Result<(), RcChecklistError> {
        if self.state == ItemState::Pending {
            return Err(RcChecklistError::NotComplete(self.id.clone()));
        }
        self.state = ItemState::SignedOff {
            reviewer: reviewer.into(),
            timestamp: timestamp.into(),
        };
        Ok(())
    }

    /// Waive the item.
    pub fn waive(&mut self, waived_by: impl Into<String>, reason: impl Into<String>) {
        self.state = ItemState::Waived {
            waived_by: waived_by.into(),
            reason: reason.into(),
        };
    }

    /// Whether this item is considered resolved (complete, signed-off, or waived).
    pub fn is_resolved(&self) -> bool {
        matches!(self.state, ItemState::Complete | ItemState::SignedOff { .. } | ItemState::Waived { .. })
    }
}

// ---------------------------------------------------------------------------
// Checklist
// ---------------------------------------------------------------------------

/// A complete release candidate validation checklist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RcChecklist {
    /// Release candidate identifier (e.g., `"v0.2.0-rc1"`).
    pub rc_id: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    items: HashMap<String, RcChecklistItem>,
    /// Ordered item IDs for deterministic iteration.
    item_order: Vec<String>,
}

impl RcChecklist {
    /// Create a new checklist for an RC.
    pub fn new(rc_id: impl Into<String>, created_at: impl Into<String>) -> Self {
        Self {
            rc_id: rc_id.into(),
            created_at: created_at.into(),
            items: HashMap::new(),
            item_order: vec![],
        }
    }

    /// Add an item.
    ///
    /// # Errors
    /// Returns [`RcChecklistError::DuplicateItem`] if the item ID already exists.
    pub fn add(&mut self, item: RcChecklistItem) -> Result<(), RcChecklistError> {
        if self.items.contains_key(&item.id) {
            return Err(RcChecklistError::DuplicateItem(item.id));
        }
        self.item_order.push(item.id.clone());
        self.items.insert(item.id.clone(), item);
        Ok(())
    }

    /// Retrieve a mutable reference to an item by ID.
    pub fn item_mut(&mut self, id: &str) -> Option<&mut RcChecklistItem> {
        self.items.get_mut(id)
    }

    /// Retrieve an immutable reference to an item by ID.
    pub fn item(&self, id: &str) -> Option<&RcChecklistItem> {
        self.items.get(id)
    }

    /// Return items in insertion order.
    pub fn items_ordered(&self) -> impl Iterator<Item = &RcChecklistItem> {
        self.item_order.iter().filter_map(|id| self.items.get(id))
    }

    /// `true` iff all blocking items are resolved.
    pub fn is_release_ready(&self) -> bool {
        self.items
            .values()
            .filter(|i| i.blocking)
            .all(|i| i.is_resolved())
    }

    /// Return unresolved blocking items.
    pub fn blocking_unresolved(&self) -> impl Iterator<Item = &RcChecklistItem> {
        self.items.values().filter(|i| i.blocking && !i.is_resolved())
    }

    /// Build the canonical infinityOS RC checklist template.
    pub fn canonical_template(rc_id: impl Into<String>, created_at: impl Into<String>) -> Self {
        let mut cl = Self::new(rc_id, created_at);

        // Testing
        cl.add(RcChecklistItem::blocking(
            "unit-tests-pass",
            RcCategory::Testing,
            "All unit test suites pass with no failures",
            "CI test job URL or cargo test output",
        )).expect("unique");
        cl.add(RcChecklistItem::blocking(
            "integration-tests-pass",
            RcCategory::Testing,
            "All integration test suites pass with no failures",
            "CI integration job URL",
        )).expect("unique");
        cl.add(RcChecklistItem::blocking(
            "coverage-thresholds-met",
            RcCategory::Testing,
            "Line coverage ≥ 80 % and branch coverage ≥ 70 % for all layers",
            "Coverage report artifact link",
        )).expect("unique");
        cl.add(RcChecklistItem::blocking(
            "contract-conformance-verified",
            RcCategory::Compatibility,
            "All IDL contract conformance probes pass",
            "Contract test run report",
        )).expect("unique");
        cl.add(RcChecklistItem::advisory(
            "fuzz-campaigns-clean",
            RcCategory::Testing,
            "No new crashes found in fuzz campaigns since previous RC",
            "Fuzz campaign summary artifact",
        )).expect("unique");

        // Performance
        cl.add(RcChecklistItem::blocking(
            "perf-no-throughput-regression",
            RcCategory::Performance,
            "Orchestrator and mesh throughput within 10 % of baseline",
            "Benchmark comparison report",
        )).expect("unique");
        cl.add(RcChecklistItem::blocking(
            "perf-no-p99-regression",
            RcCategory::Performance,
            "p99 latency within 20 % of baseline for all measured paths",
            "Benchmark comparison report",
        )).expect("unique");

        // Security
        cl.add(RcChecklistItem::blocking(
            "sast-scan-clean",
            RcCategory::Security,
            "SAST pipeline (cargo-audit, cargo-deny, semgrep, CodeQL) reports no critical/high findings",
            "SAST scan report artifact",
        )).expect("unique");
        cl.add(RcChecklistItem::blocking(
            "dast-scan-clean",
            RcCategory::Security,
            "DAST pipeline reports no high-risk findings against local API surface",
            "DAST scan report artifact",
        )).expect("unique");
        cl.add(RcChecklistItem::advisory(
            "sbom-generated",
            RcCategory::Security,
            "SBOM generated and attached as a release artifact",
            "SBOM artifact link",
        )).expect("unique");

        // Documentation
        cl.add(RcChecklistItem::blocking(
            "changelog-updated",
            RcCategory::Documentation,
            "CHANGELOG entry written for this release",
            "CHANGELOG.md diff link",
        )).expect("unique");
        cl.add(RcChecklistItem::advisory(
            "api-docs-regenerated",
            RcCategory::Documentation,
            "Rust API docs regenerated with `cargo doc` and reviewed for completeness",
            "cargo doc output / docs.rs preview",
        )).expect("unique");

        // Operations
        cl.add(RcChecklistItem::blocking(
            "runbook-reviewed",
            RcCategory::Operations,
            "Operational runbook reviewed and updated for this release",
            "Runbook diff link",
        )).expect("unique");

        // Compatibility
        cl.add(RcChecklistItem::blocking(
            "abi-compatibility-confirmed",
            RcCategory::Compatibility,
            "Kernel ABI version negotiation tested against all supported runtime versions",
            "ABI conformance test run output",
        )).expect("unique");

        cl
    }

    /// Render the checklist as a Markdown report.
    pub fn to_markdown(&self) -> String {
        let mut lines = vec![
            format!("# Release Candidate Checklist — {}", self.rc_id),
            format!("_Created: {}_", self.created_at),
            String::new(),
            format!("**Release ready:** {}", if self.is_release_ready() { "✅ YES" } else { "❌ NO" }),
            String::new(),
        ];

        let categories = [
            RcCategory::Testing,
            RcCategory::Performance,
            RcCategory::Security,
            RcCategory::Documentation,
            RcCategory::Compatibility,
            RcCategory::Operations,
            RcCategory::Legal,
        ];

        for cat in &categories {
            let cat_items: Vec<_> = self
                .items_ordered()
                .filter(|i| i.category == *cat)
                .collect();
            if cat_items.is_empty() {
                continue;
            }
            lines.push(format!("## {:?}", cat));
            for item in cat_items {
                let marker = match &item.state {
                    ItemState::Pending => "[ ]",
                    ItemState::Complete => "[~]",
                    ItemState::SignedOff { .. } => "[x]",
                    ItemState::Waived { .. } => "[w]",
                };
                let blocking_tag = if item.blocking { " _(blocking)_" } else { "" };
                lines.push(format!("- {} **{}**{}: {}", marker, item.id, blocking_tag, item.description));
                if let Some(link) = &item.evidence_link {
                    lines.push(format!("  - Evidence: {link}"));
                }
            }
            lines.push(String::new());
        }

        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_template_is_not_release_ready_when_all_pending() {
        let cl = RcChecklist::canonical_template("v0.1.0-rc1", "2026-03-12T00:00:00Z");
        assert!(!cl.is_release_ready());
    }

    #[test]
    fn resolving_all_blocking_items_makes_release_ready() {
        let mut cl = RcChecklist::canonical_template("v0.1.0-rc1", "2026-03-12T00:00:00Z");
        let blocking_ids: Vec<_> = cl
            .items_ordered()
            .filter(|i| i.blocking)
            .map(|i| i.id.clone())
            .collect();

        for id in &blocking_ids {
            cl.item_mut(id).unwrap().mark_complete("https://ci.example.com/job/123");
        }
        assert!(cl.is_release_ready());
    }

    #[test]
    fn markdown_render_contains_rc_id() {
        let cl = RcChecklist::canonical_template("v0.2.0-rc2", "2026-03-12T00:00:00Z");
        let md = cl.to_markdown();
        assert!(md.contains("v0.2.0-rc2"));
        assert!(md.contains("unit-tests-pass"));
    }

    #[test]
    fn duplicate_item_is_rejected() {
        let mut cl = RcChecklist::new("rc-test", "t");
        cl.add(RcChecklistItem::blocking("item-1", RcCategory::Testing, "d", "e")).unwrap();
        assert!(cl.add(RcChecklistItem::blocking("item-1", RcCategory::Testing, "d", "e")).is_err());
    }

    #[test]
    fn sign_off_on_pending_item_errors() {
        let mut cl = RcChecklist::canonical_template("rc-x", "t");
        let item = cl.item_mut("unit-tests-pass").unwrap();
        assert!(item.sign_off("reviewer", "2026-03-12").is_err());
    }

    #[test]
    fn waived_item_counts_as_resolved() {
        let mut item = RcChecklistItem::blocking("b", RcCategory::Testing, "d", "e");
        item.waive("release-eng", "approved for this release");
        assert!(item.is_resolved());
    }
}
