//! # ify-quality — Quality Engineering
//!
//! This crate implements the full **Epic Q** feature set for infinityOS.
//! It provides the quality engineering substrate: test strategy, deterministic
//! test datasets, quality gates, fuzz infrastructure, security testing
//! pipelines, contract tests, golden tests, load-test scenarios, test
//! reporting widgets, and the release candidate validation checklist.
//!
//! ## Module map
//!
//! | Module | Epic Q item |
//! |--------|-------------|
//! | [`strategy`] | Unit/integration/performance test strategy (item 1) |
//! | [`datasets`] | Deterministic test datasets for graph/data paths (item 2) |
//! | [`gates`] | Quality gates for merge readiness (item 3) |
//! | [`fuzz`] | Fuzz testing infrastructure for parsers/serializers (item 4) |
//! | [`security_testing`] | SAST/DAST pipeline definitions (item 5) |
//! | [`contract`] | Contract tests for cross-layer interfaces / IDL (item 6) |
//! | [`golden`] | Golden tests for UI layouts (item 7) |
//! | [`load`] | Load tests for orchestrator and mesh (item 8) |
//! | [`report`] | Test reporting widget (item 9) |
//! | [`rc_checklist`] | Release candidate validation checklist (item 10) |
//!
//! ## Quick start
//!
//! ```rust
//! use ify_quality::{
//!     strategy::TestStrategy,
//!     datasets::{GraphFixture, DatasetFixture},
//!     gates::{QualityGateSet, MergeReadinessReport},
//!     fuzz::FuzzRegistry,
//!     security_testing::SecurityPipeline,
//!     contract::ContractRegistry,
//!     golden::{GoldenStore, CanvasLayoutFixtures},
//!     load::{LoadScenarioRegistry, LoadRunner},
//!     report::{TestReportSnapshot, TestReportWidget},
//!     rc_checklist::RcChecklist,
//! };
//!
//! // 1 — Test strategy
//! let strategy = TestStrategy::canonical();
//! assert!(!strategy.suites().is_empty());
//!
//! // 2 — Deterministic datasets
//! let graph = GraphFixture::linear_three();
//! assert_eq!(graph.nodes.len(), 3);
//! let dataset = DatasetFixture::timeseries_five();
//! assert_eq!(dataset.records.len(), 5);
//!
//! // 3 — Quality gates
//! let gates = QualityGateSet::canonical();
//! let mut report = MergeReadinessReport::new();
//! report.record("unit_tests_passed", 1.0);
//! report.record("integration_tests_passed", 1.0);
//! report.record("line_coverage_pct", 85.0);
//! report.record("branch_coverage_pct", 72.0);
//! report.record("critical_security_findings", 0.0);
//! report.record("high_security_findings", 0.0);
//! report.record("p99_latency_regression_pct", 5.0);
//! report.record("throughput_regression_pct", 3.0);
//! report.record("contract_conformance_passed", 1.0);
//! report.record("changelog_entry_present", 1.0);
//! let verdict = gates.evaluate_all(&report);
//! assert!(verdict.all_passed);
//!
//! // 4 — Fuzz registry
//! let fuzz = FuzzRegistry::canonical();
//! assert!(fuzz.get("fuzz_json_deserializer").is_some());
//!
//! // 5 — Security pipeline
//! let pipeline = SecurityPipeline::canonical();
//! assert!(pipeline.get("cargo-audit").is_some());
//!
//! // 6 — Contract registry
//! let contracts = ContractRegistry::canonical();
//! assert!(contracts.get("mesh-artifact-api-v1").is_some());
//!
//! // 7 — Golden tests
//! let mut store = GoldenStore::new();
//! let node = CanvasLayoutFixtures::single_node_standard_zoom();
//! let snapshot = node.to_snapshot_string();
//! store.update("single-node-standard-zoom", &snapshot);
//! assert!(store.assert_matches("single-node-standard-zoom", &snapshot).is_ok());
//!
//! // 8 — Load scenarios
//! let load_reg = LoadScenarioRegistry::canonical();
//! let scenario = load_reg.get("orchestrator-single-threaded-baseline").unwrap();
//! let result = LoadRunner::run_noop(scenario);
//! assert_eq!(result.ops_completed, scenario.total_ops);
//!
//! // 9 — Test reporting widget
//! let snap = TestReportSnapshot::new("sha-abc123", "2026-03-12T00:00:00Z");
//! let rendered = TestReportWidget::render(&snap);
//! assert!(rendered.contains("sha-abc123"));
//!
//! // 10 — RC checklist
//! let cl = RcChecklist::canonical_template("v0.1.0-rc1", "2026-03-12T00:00:00Z");
//! assert!(!cl.is_release_ready()); // all items still pending
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod contract;
pub mod datasets;
pub mod fuzz;
pub mod gates;
pub mod golden;
pub mod load;
pub mod rc_checklist;
pub mod report;
pub mod security_testing;
pub mod strategy;

// ---------------------------------------------------------------------------
// Crate-level re-exports
// ---------------------------------------------------------------------------

// strategy
pub use strategy::{
    CoverageThreshold,
    PyramidLevel,
    StrategyError,
    TestedLayer,
    TestStrategy,
    TestSuiteDescriptor,
    TimingBudget,
};

// datasets
pub use datasets::{
    DataRecord,
    DatasetFixture,
    FixtureEdge,
    FixtureNode,
    GraphFixture,
};

// gates
pub use gates::{
    GateCategory,
    GateError,
    GateOp,
    GateOutcome,
    GateSetVerdict,
    MergeReadinessReport,
    QualityGate,
    QualityGateSet,
};

// fuzz
pub use fuzz::{
    CorpusEntry,
    FuzzCategory,
    FuzzError,
    FuzzRegistry,
    FuzzTarget,
};

// security_testing
pub use security_testing::{
    FindingSeverity,
    PipelineEvaluation,
    PipelineKind,
    ScannerConfig,
    SecurityFinding,
    SecurityPipeline,
    SecurityTestingError,
};

// contract
pub use contract::{
    ContractError,
    ContractInvariant,
    ContractRegistry,
    ContractRunSummary,
    ContractTestRunner,
    InterfaceContract,
    InterfaceLayer,
    ProbeResult,
};

// golden
pub use golden::{
    CanvasLayoutFixtures,
    GoldenError,
    GoldenStore,
    LayoutNode,
};

// load
pub use load::{
    LatencyHistogram,
    LoadError,
    LoadResult,
    LoadRunner,
    LoadScenario,
    LoadScenarioRegistry,
    LoadTarget,
};

// report
pub use report::{
    FuzzCampaignSummary,
    ReportError,
    SecurityScanSummary,
    SuiteReport,
    SuiteStatus,
    TestReportSnapshot,
    TestReportWidget,
};

// rc_checklist
pub use rc_checklist::{
    ItemState,
    RcCategory,
    RcChecklist,
    RcChecklistError,
    RcChecklistItem,
};
