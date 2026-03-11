//! # ify-reliability — Kaizen Reliability Loop
//!
//! This crate implements the full **Epic K** feature set for infinityOS.
//! It provides the infrastructure for continuous reliability improvement:
//! SLO tracking, metrics (MTTR, error budget, regression rate), chaos testing,
//! automated incident management, regression triage, and a widget-ready
//! reliability dashboard.
//!
//! ## Module map
//!
//! | Module | Epic K item |
//! |--------|-------------|
//! | [`metrics`] | MTTR, error budget, and regression rate tracking (item 2) |
//! | [`slo`] | SLOs for task execution and UI responsiveness (item 4) |
//! | [`chaos`] | Chaos testing for replication kernel and orchestrator (item 5) |
//! | [`incident`] | Automated incident creation from telemetry signals (item 7) |
//! | [`regression`] | Regression triage workflow: labels, owners, SLA (item 8) |
//! | [`dashboard`] | Reliability dashboard integrated in widgets (item 10) |
//!
//! Items 1 (review cadence), 3 (measurable improvement), 6 (runbooks), and
//! 9 (postmortem template) are delivered as documentation in
//! `docs/reliability/`.
//!
//! ## Quick start
//!
//! ```rust
//! use ify_reliability::{
//!     metrics::ReliabilityMetrics,
//!     slo::{SloRegistry, SloSample},
//!     incident::{IncidentPipeline, IncidentRule, TelemetrySignal},
//!     metrics::IncidentSeverity,
//!     regression::{RegressionTriageEngine, RegressionReport},
//!     dashboard::ReliabilityDashboard,
//! };
//!
//! // --- Metrics ---
//! let mut metrics = ReliabilityMetrics::new();
//! let snap = metrics.snapshot();
//! assert!(snap.get("task_error_budget_remaining").is_some());
//!
//! // --- SLOs ---
//! let mut slo_reg = SloRegistry::with_defaults();
//! slo_reg
//!     .record(SloSample {
//!         slo_name: "task.p99_latency_ms".into(),
//!         value: 150.0,
//!         timestamp_secs: 0,
//!     })
//!     .unwrap();
//! let status = slo_reg.evaluate("task.p99_latency_ms").unwrap();
//! assert!(status.passing);
//!
//! // --- Incident pipeline ---
//! let mut pipeline = IncidentPipeline::new();
//! pipeline.add_rule(IncidentRule::immediate(
//!     "slo-breach",
//!     "slo.",
//!     "SLO breach: {signal_type}",
//!     IncidentSeverity::High,
//! )).unwrap();
//! let signal = TelemetrySignal::new("slo.breach", "dim-1", serde_json::json!({}));
//! let affected = pipeline.ingest(signal);
//! assert_eq!(affected.len(), 1);
//!
//! // --- Regression triage ---
//! let mut triage = RegressionTriageEngine::new();
//! let report = RegressionReport::new(
//!     "p99 latency regression",
//!     "scheduler",
//!     "2026-W11",
//!     300.0, // measured
//!     200.0, // baseline
//! );
//! let id = triage.submit(report).unwrap();
//! assert!(triage.get(&id).is_some());
//!
//! // --- Dashboard ---
//! let mut dashboard = ReliabilityDashboard::new();
//! let snapshot = dashboard.snapshot(&slo_reg, &metrics, &[], &[]);
//! assert!(snapshot.panel("slo_status").is_some());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod chaos;
pub mod dashboard;
pub mod incident;
pub mod metrics;
pub mod regression;
pub mod slo;

// ---------------------------------------------------------------------------
// Crate-level re-exports
// ---------------------------------------------------------------------------

// metrics
pub use metrics::{
    CycleEntry,
    ErrorBudget,
    IncidentRecord,
    IncidentSeverity,
    MetricsError,
    MttrTracker,
    ReliabilityMetrics,
    RegressionTracker,
};

// slo
pub use slo::{
    SloError,
    SloKind,
    SloRegistry,
    SloSample,
    SloStatus,
    SloThreshold,
};

// chaos
pub use chaos::{
    ChaosDecision,
    ChaosEngine,
    ChaosError,
    ChaosPolicy,
    ChaosScenario,
    FaultKind,
};

// incident
pub use incident::{
    Incident,
    IncidentError,
    IncidentPipeline,
    IncidentRule,
    IncidentState,
    TelemetrySignal,
};

// regression
pub use regression::{
    OwnerRule,
    RegressionReport,
    RegressionTriageEngine,
    SlaTier,
    TriageError,
    TriageLabel,
    TriageOwner,
    TriageState,
};

// dashboard
pub use dashboard::{
    DashboardEvent,
    DashboardEventKind,
    DashboardPanel,
    DashboardSnapshot,
    PanelHealth,
    ReliabilityDashboard,
};
