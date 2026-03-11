# Postmortem Template

**Epic K — Item 9**

Use this template for every P0 and P1 incident after it is resolved.  Postmortems are blameless.

---

## Postmortem: \<Incident Title\>

**Incident ID**: `<incident-id>`  
**Severity**: `<P0 | P1>`  
**Date**: `<YYYY-MM-DD>`  
**Author(s)**: `<names>`  
**Status**: `<Draft | In Review | Published>`

---

## Summary

_One or two sentences describing what happened, its impact, and how it was resolved._

---

## Timeline

| Time (UTC) | Event |
|------------|-------|
| HH:MM | Incident opened by telemetry signal / alert. |
| HH:MM | Oncall engineer acknowledged. |
| HH:MM | Root cause identified. |
| HH:MM | Mitigation applied. |
| HH:MM | Incident resolved; SLO recovery confirmed. |

---

## Root Cause

_Describe the root cause in detail.  Include the component, the triggering condition, and any contributing factors._

---

## Impact

| Dimension | Duration | Users / Tasks Affected |
|-----------|----------|------------------------|
| `<dim-id>` | `<N min>` | `<count>` |

- **SLO breached**: `<yes/no>` — `<SLO name>` compliance dropped to `<X %>`
- **Error budget consumed**: `<Y seconds>` of the `<SLO name>` 30-day budget

---

## Detection

_How was the incident detected?  Was it automated (telemetry signal → incident pipeline) or manual?  How long did it take from onset to detection?_

---

## Mitigation

_What actions were taken to mitigate the incident?_

---

## Resolution

_What resolved the incident permanently?_

---

## What Went Well

- _Item 1_
- _Item 2_

---

## What Went Wrong

- _Item 1_
- _Item 2_

---

## Action Items

| Action | Owner | Due Date | Tracking Issue |
|--------|-------|----------|----------------|
| _Add runbook entry for this failure mode._ | | | |
| _Add chaos test to reproduce this scenario._ | | | |

---

## Publishing Workflow

1. Author fills in this template after incident resolution.
2. Postmortem is reviewed by the cycle owner and SLO steward within 48 hours of incident resolution.
3. Approved postmortem is committed to `docs/reliability/postmortems/<YYYY-MM-DD>-<incident-id>.md`.
4. A link to the published postmortem is added to the incident record in the `IncidentPipeline`.
5. Action items are tracked in the repository issue tracker with the `reliability` label.
6. Action items are reviewed in the next weekly Kaizen review cycle.

---

## References

- Incident pipeline: [`ify-reliability::incident`](../../runtime-rust/crates/ify-reliability/src/incident.rs)
- Review cadence: [`docs/reliability/review-cadence.md`](review-cadence.md)
- Runbooks: [`docs/reliability/runbooks/`](runbooks/)
