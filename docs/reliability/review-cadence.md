# Reliability Review Cadence

**Epic K — Item 1**

## Overview

infinityOS follows a weekly **Kaizen Reliability Review** cadence to continuously measure and improve system reliability.  Every cycle produces one measurable reliability improvement.

## Schedule

| Day | Activity |
|-----|----------|
| **Monday** | Pull the prior-week SLO status report from the dashboard. Identify SLOs that were breaching or at-risk. |
| **Tuesday** | Triage open regressions and incidents from the prior week using [`RegressionTriageEngine`](../../runtime-rust/crates/ify-reliability/src/regression.rs) and the incident pipeline. |
| **Wednesday** | Select one improvement item from the backlog, assign owner and SLA tier. |
| **Thursday** | Implement the improvement; update baseline metrics and budgets. |
| **Friday** | Verify the improvement is measurable in the dashboard; publish a summary artifact. |

## Metrics Reviewed Each Cycle

1. **MTTR** (Mean Time To Recovery) — target < 30 min for P0, < 4 h for P1.
2. **Error budget remaining** — task SLO target: 99.9 %, UI target: 99.5 %.
3. **Regression rate** — target: ≤ 1 new regression per cycle.
4. **SLO compliance** — all five default SLOs should be passing.
5. **Open incident count** — target: zero open P0/P1 incidents at end of cycle.

## Review Artifacts

Each cycle produces:

- A dashboard snapshot saved as a mesh artifact at `reliability/review/<cycle_id>/snapshot.json`.
- A regression summary saved at `reliability/review/<cycle_id>/regressions.md`.
- One improvement commit or PR linked in the cycle notes.

## Roles

| Role | Responsibility |
|------|---------------|
| **Cycle owner** | Rotates weekly; coordinates Monday–Friday activities. |
| **SLO steward** | Maintains SLO thresholds and error budget targets. |
| **Oncall engineer** | Triages new incidents; owns P0/P1 response. |
| **Reliability agent** | Runs automated chaos tests; feeds telemetry signals into the incident pipeline. |

## Improvement Selection Criteria

1. Impact: highest regression ratio or most SLO-budget consumed.
2. Feasibility: implementable within one cycle (Thursday slot).
3. Measurability: improvement must produce a visible delta in the dashboard.

## References

- `docs/reliability/runbooks/` — runbooks for common failures.
- `docs/reliability/postmortem-template.md` — postmortem template for P0/P1 incidents.
- [`ify-reliability`](../../runtime-rust/crates/ify-reliability/src/lib.rs) — Rust implementation.
