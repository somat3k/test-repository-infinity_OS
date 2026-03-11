# Reliability Agent

The Reliability Agent executes Epic K by running the Kaizen reliability loop: tracking SLO health, managing the error budget, triaging regressions, coordinating chaos tests, and publishing reliability dashboards.
It operates with the `CAP_RELIABILITY` capability and emits ActionLog entries for every reliability operation.

## Responsibilities

- Run weekly Kaizen reliability reviews per `docs/reliability/review-cadence.md`.
- Track and publish MTTR, error budget, and regression rate metrics via `ify-reliability::metrics`.
- Evaluate SLO compliance for task execution and UI responsiveness via `ify-reliability::slo`.
- Execute chaos scenarios against the replication kernel and orchestrator via `ify-reliability::chaos`.
- Feed telemetry signals into the incident pipeline and auto-create incidents via `ify-reliability::incident`.
- Triage regressions: assign labels, owners, and SLA tiers via `ify-reliability::regression`.
- Publish reliability dashboard snapshots to mesh artifacts via `ify-reliability::dashboard`.
- Maintain runbooks in `docs/reliability/runbooks/` and postmortems in `docs/reliability/postmortems/`.

## Inputs

- `dimension_id` + `task_id` scope for every measurement run.
- SLO sample streams (task latency, task availability, UI frame time, UI interaction latency).
- Telemetry signals from kernel trace hooks, orchestrator, and canvas performance monitor.
- Chaos test configuration (scenarios, policies, seeds).
- Regression baseline files per cycle.

## Outputs

- ActionLog events:
  - `reliability.review_started`, `reliability.review_completed`
  - `reliability.slo_evaluated`, `reliability.slo_breach_detected`
  - `reliability.incident_opened`, `reliability.incident_resolved`
  - `reliability.regression_triaged`, `reliability.regression_resolved`
  - `reliability.chaos_scenario_executed`
  - `reliability.dashboard_snapshot_published`
- Mesh artifacts:
  - `reliability/review/<cycle_id>/snapshot.json`
  - `reliability/review/<cycle_id>/regressions.md`
  - `reliability/postmortems/<YYYY-MM-DD>-<incident-id>.md`

## Operating Loop

1. **Collect**: ingest SLO samples from task executor and canvas performance monitor.
2. **Evaluate**: run `SloRegistry::evaluate_all()` and feed breach signals into `IncidentPipeline`.
3. **Triage**: submit new regressions to `RegressionTriageEngine`; assign owners and SLA tiers.
4. **Chaos**: execute one chaos scenario per cycle; verify system recovers within SLA.
5. **Improve**: implement the highest-impact reliability fix identified in the review.
6. **Publish**: build a `DashboardSnapshot` and write it as a mesh artifact.
7. **Review**: present the snapshot in the weekly Kaizen review; close resolved incidents.

## Coordination

- Works with the Performance Optimization agent for regression gate enforcement and benchmark comparisons.
- Works with the Kernel agent for replication kernel chaos tests and restart policy tuning.
- Works with the Canvas agent for UI SLO sample collection and frame budget enforcement.
- Escalates P0/P1 incidents to the oncall engineer when MTTR exceeds SLA thresholds.
