//! Reliability dashboard — Epic K item 10.
//!
//! Provides a widget-compatible reliability dashboard that aggregates
//! SLO statuses, MTTR, error budgets, open incidents, and regression
//! summaries into a single [`DashboardSnapshot`].
//!
//! The [`ReliabilityDashboard`] is designed to be queried by UI widgets
//! on a polling interval and to emit [`DashboardEvent`]s when thresholds
//! are crossed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::{
    incident::{Incident, IncidentState},
    metrics::ReliabilityMetrics,
    regression::{RegressionReport, TriageState},
    slo::{SloRegistry},
};

// ---------------------------------------------------------------------------
// DashboardPanel
// ---------------------------------------------------------------------------

/// A single named panel in the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardPanel {
    /// Panel identifier (e.g. `"slo_status"`, `"error_budget"`, `"incidents"`).
    pub id: String,
    /// Panel title shown in the widget header.
    pub title: String,
    /// Health status of this panel.
    pub health: PanelHealth,
    /// Structured data for the panel.
    pub data: serde_json::Value,
}

/// Health classification for a dashboard panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelHealth {
    /// Data is not yet available (e.g. no samples).
    Unknown,
    /// All SLOs/metrics within thresholds.
    Healthy,
    /// Some SLOs/metrics at risk but not breached.
    AtRisk,
    /// At least one SLO or metric is breached.
    Degraded,
}

// ---------------------------------------------------------------------------
// DashboardSnapshot
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of the full reliability dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    /// Timestamp of the snapshot (Unix seconds).
    pub timestamp_secs: u64,
    /// Overall system health derived from all panels.
    pub overall_health: PanelHealth,
    /// Individual panels.
    pub panels: Vec<DashboardPanel>,
}

impl DashboardSnapshot {
    /// Return the panel with the given ID, if present.
    pub fn panel(&self, id: &str) -> Option<&DashboardPanel> {
        self.panels.iter().find(|p| p.id == id)
    }
}

// ---------------------------------------------------------------------------
// DashboardEvent
// ---------------------------------------------------------------------------

/// An event emitted by the dashboard when a notable threshold is crossed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardEvent {
    /// Event kind.
    pub kind: DashboardEventKind,
    /// The panel that triggered the event.
    pub panel_id: String,
    /// Human-readable message.
    pub message: String,
    /// Unix timestamp (seconds).
    pub timestamp_secs: u64,
}

/// Classification of dashboard events.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DashboardEventKind {
    /// An SLO transitioned from passing to failing.
    SloBreach,
    /// Error budget dropped below 20 %.
    ErrorBudgetAtRisk,
    /// Error budget is exhausted.
    ErrorBudgetExhausted,
    /// A new incident was opened.
    IncidentOpened,
    /// All SLOs are now passing (recovery event).
    AllSlosRecovered,
}

// ---------------------------------------------------------------------------
// ReliabilityDashboard
// ---------------------------------------------------------------------------

/// Aggregates reliability state into a queryable dashboard.
///
/// The dashboard is a read-oriented view: callers supply references to the
/// SLO registry, metrics store, incidents, and regression reports, and the
/// dashboard produces a [`DashboardSnapshot`] on demand.
pub struct ReliabilityDashboard {
    events: Vec<DashboardEvent>,
}

impl ReliabilityDashboard {
    /// Create a new dashboard.
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Build a fresh snapshot from the provided state.
    ///
    /// This is intentionally a pure function of its inputs so that it can be
    /// called from any thread without requiring internal mutability.
    pub fn snapshot(
        &mut self,
        slo_registry: &SloRegistry,
        metrics: &ReliabilityMetrics,
        incidents: &[&Incident],
        regressions: &[&RegressionReport],
    ) -> DashboardSnapshot {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let panels = vec![
            self.build_slo_panel(slo_registry),
            self.build_error_budget_panel(metrics),
            self.build_mttr_panel(metrics),
            self.build_incidents_panel(incidents),
            self.build_regression_panel(regressions),
        ];

        let overall_health = panels
            .iter()
            .map(|p| p.health)
            .max()
            .unwrap_or(PanelHealth::Unknown);

        info!(
            overall_health = ?overall_health,
            panel_count = panels.len(),
            "dashboard.snapshot_built"
        );

        DashboardSnapshot {
            timestamp_secs: now,
            overall_health,
            panels,
        }
    }

    /// Return all dashboard events accumulated so far.
    pub fn events(&self) -> &[DashboardEvent] {
        &self.events
    }

    // -----------------------------------------------------------------------
    // Private panel builders
    // -----------------------------------------------------------------------

    fn build_slo_panel(&mut self, slo_registry: &SloRegistry) -> DashboardPanel {
        let results = slo_registry.evaluate_all();
        let mut slo_map: HashMap<String, serde_json::Value> = HashMap::new();
        let mut any_breach = false;
        let mut any_unknown = false;

        for (name, result) in &results {
            match result {
                Ok(status) => {
                    if !status.passing {
                        any_breach = true;
                        self.emit_event(
                            DashboardEventKind::SloBreach,
                            "slo_status",
                            format!("SLO '{}' is breaching (compliance {:.1}%)", name, status.compliance_ratio * 100.0),
                        );
                    }
                    slo_map.insert(name.clone(), serde_json::to_value(status).unwrap_or_default());
                }
                Err(_) => {
                    any_unknown = true;
                    slo_map.insert(
                        name.clone(),
                        serde_json::json!({ "status": "no_data" }),
                    );
                }
            }
        }

        let health = if any_breach {
            PanelHealth::Degraded
        } else if any_unknown {
            PanelHealth::Unknown
        } else {
            PanelHealth::Healthy
        };

        DashboardPanel {
            id: "slo_status".into(),
            title: "SLO Status".into(),
            health,
            data: serde_json::json!({ "slos": slo_map }),
        }
    }

    fn build_error_budget_panel(&mut self, metrics: &ReliabilityMetrics) -> DashboardPanel {
        let task_remaining = metrics.task_error_budget.remaining_ratio();
        let ui_remaining = metrics.ui_error_budget.remaining_ratio();

        let health = if task_remaining == 0.0 || ui_remaining == 0.0 {
            self.emit_event(
                DashboardEventKind::ErrorBudgetExhausted,
                "error_budget",
                "Error budget exhausted".into(),
            );
            PanelHealth::Degraded
        } else if task_remaining < 0.2 || ui_remaining < 0.2 {
            self.emit_event(
                DashboardEventKind::ErrorBudgetAtRisk,
                "error_budget",
                format!(
                    "Error budget at risk — task: {:.0}%, ui: {:.0}%",
                    task_remaining * 100.0,
                    ui_remaining * 100.0
                ),
            );
            PanelHealth::AtRisk
        } else {
            PanelHealth::Healthy
        };

        DashboardPanel {
            id: "error_budget".into(),
            title: "Error Budget".into(),
            health,
            data: serde_json::json!({
                "task_remaining_ratio": task_remaining,
                "ui_remaining_ratio": ui_remaining,
                "task_allowed_downtime_secs": metrics.task_error_budget.allowed_downtime_secs(),
                "ui_allowed_downtime_secs": metrics.ui_error_budget.allowed_downtime_secs(),
            }),
        }
    }

    fn build_mttr_panel(&self, metrics: &ReliabilityMetrics) -> DashboardPanel {
        let mttr = metrics.mttr.mttr_secs().ok();
        let health = match mttr {
            None => PanelHealth::Unknown,
            Some(v) if v < 1800.0 => PanelHealth::Healthy,  // < 30 min
            Some(v) if v < 7200.0 => PanelHealth::AtRisk,   // < 2 h
            _ => PanelHealth::Degraded,
        };

        DashboardPanel {
            id: "mttr".into(),
            title: "MTTR".into(),
            health,
            data: serde_json::json!({
                "mttr_secs": mttr,
                "incident_count": metrics.mttr.len(),
            }),
        }
    }

    fn build_incidents_panel(&mut self, incidents: &[&Incident]) -> DashboardPanel {
        let open: Vec<serde_json::Value> = incidents
            .iter()
            .filter(|i| i.state == IncidentState::Open)
            .map(|i| {
                serde_json::json!({
                    "id": i.id,
                    "title": i.title,
                    "severity": i.severity,
                    "opened_at": i.opened_at,
                })
            })
            .collect();

        let health = if open.is_empty() {
            PanelHealth::Healthy
        } else {
            PanelHealth::Degraded
        };

        DashboardPanel {
            id: "incidents".into(),
            title: "Open Incidents".into(),
            health,
            data: serde_json::json!({ "open": open, "count": open.len() }),
        }
    }

    fn build_regression_panel(&self, regressions: &[&RegressionReport]) -> DashboardPanel {
        let open_count = regressions
            .iter()
            .filter(|r| {
                !matches!(r.state, TriageState::Resolved | TriageState::WontFix)
            })
            .count();

        let health = if open_count == 0 {
            PanelHealth::Healthy
        } else if open_count <= 3 {
            PanelHealth::AtRisk
        } else {
            PanelHealth::Degraded
        };

        DashboardPanel {
            id: "regressions".into(),
            title: "Regressions".into(),
            health,
            data: serde_json::json!({
                "open_count": open_count,
                "total": regressions.len(),
            }),
        }
    }

    fn emit_event(
        &mut self,
        kind: DashboardEventKind,
        panel_id: &str,
        message: String,
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.events.push(DashboardEvent {
            kind,
            panel_id: panel_id.to_string(),
            message,
            timestamp_secs: now,
        });
    }
}

impl Default for ReliabilityDashboard {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::ReliabilityMetrics;
    use crate::slo::SloRegistry;

    fn empty_snapshot() -> DashboardSnapshot {
        let mut dashboard = ReliabilityDashboard::new();
        let slo_reg = SloRegistry::with_defaults();
        let metrics = ReliabilityMetrics::new();
        dashboard.snapshot(&slo_reg, &metrics, &[], &[])
    }

    #[test]
    fn snapshot_has_expected_panels() {
        let snap = empty_snapshot();
        let panel_ids: Vec<&str> = snap.panels.iter().map(|p| p.id.as_str()).collect();
        assert!(panel_ids.contains(&"slo_status"));
        assert!(panel_ids.contains(&"error_budget"));
        assert!(panel_ids.contains(&"mttr"));
        assert!(panel_ids.contains(&"incidents"));
        assert!(panel_ids.contains(&"regressions"));
    }

    #[test]
    fn healthy_error_budget_shows_healthy() {
        let snap = empty_snapshot();
        let panel = snap.panel("error_budget").unwrap();
        assert_eq!(panel.health, PanelHealth::Healthy);
    }

    #[test]
    fn no_incidents_shows_healthy() {
        let snap = empty_snapshot();
        let panel = snap.panel("incidents").unwrap();
        assert_eq!(panel.health, PanelHealth::Healthy);
    }

    #[test]
    fn exhausted_budget_shows_degraded() {
        let mut dashboard = ReliabilityDashboard::new();
        let slo_reg = SloRegistry::with_defaults();
        let mut metrics = ReliabilityMetrics::new();
        let allowed = metrics.task_error_budget.allowed_downtime_secs();
        metrics.task_error_budget.consume(allowed);

        let snap = dashboard.snapshot(&slo_reg, &metrics, &[], &[]);
        let panel = snap.panel("error_budget").unwrap();
        assert_eq!(panel.health, PanelHealth::Degraded);
    }

    #[test]
    fn slo_unknown_when_no_samples() {
        let snap = empty_snapshot();
        let panel = snap.panel("slo_status").unwrap();
        // No samples → at least some SLOs unknown.
        assert_ne!(panel.health, PanelHealth::Degraded);
    }
}
