# Runbook: Error Budget Exhaustion

**Epic K — Item 6**

**Applies to**: `task.availability` and `ui.frame_time_ms` error budgets.

---

## Symptom

The `error_budget` panel in the reliability dashboard shows `Degraded` (exhausted) or `AtRisk` (< 20 % remaining).  A `DashboardEvent` with kind `ErrorBudgetExhausted` or `ErrorBudgetAtRisk` has been emitted.

---

## Immediate Actions (< 5 min)

1. **Identify which budget is exhausted** (task or UI) from the dashboard panel data.
2. **Stop all non-critical chaos tests** for the affected SLO:
   ```rust
   // Deactivate all active chaos scenarios
   for scenario in engine.snapshot()? {
       if scenario.active { engine.deactivate(&scenario.id)?; }
   }
   ```
3. **Freeze optional feature rollouts** that touch the affected subsystem.
4. **Notify stakeholders** that SLO headroom is gone for the remainder of the window.

---

## Diagnosis

| Budget exhausted | Common Causes |
|-----------------|---------------|
| `task` budget | Incident(s) with long MTTR, retry storms, replica crashes |
| `ui` budget | Repeated frame budget violations, canvas rendering regressions |

Review `MttrTracker` history and `RegressionTracker` cycles to identify the largest contributors.

---

## Mitigation

1. **Prioritise P0/P1 incidents**: every minute of open incident consumes budget.
2. **Reduce MTTR**: streamline detection → acknowledgement → resolution pipeline.
3. **Address top regression**: pick the regression with the highest `regression_ratio()` and fix it.
4. **Reset error budget** only at the start of a new 30-day window via `ErrorBudget::reset()`.

---

## Recovery Verification

1. Confirm `ErrorBudget::remaining_ratio() > 0.2` after improvements.
2. Confirm the dashboard `error_budget` panel shows `Healthy` or `AtRisk`.
3. Document the budget consumption event in the weekly Kaizen review notes.

---

## Escalation

- If budget exhausts in < 3 weeks: convene an emergency reliability review.
- If a single incident consumed > 50 % of budget: mandatory postmortem.

---

## References

- Error budget model: [`ify-reliability::metrics::ErrorBudget`](../../runtime-rust/crates/ify-reliability/src/metrics.rs)
- Dashboard: [`ify-reliability::dashboard`](../../runtime-rust/crates/ify-reliability/src/dashboard.rs)
- Review cadence: [`docs/reliability/review-cadence.md`](../review-cadence.md)
