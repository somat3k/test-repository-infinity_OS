# Runbook: Task Execution SLO Breach

**Epic K — Item 6**

**Applies to**: `task.p99_latency_ms`, `task.p50_latency_ms`, `task.availability` SLOs

---

## Symptom

The reliability dashboard shows `task.p99_latency_ms` or `task.availability` in a **Degraded** state.  An incident may have been opened automatically by the incident pipeline with signal type `slo.breach`.

---

## Immediate Actions (< 5 min)

1. **Check open incident count** in the dashboard `incidents` panel.
2. **Identify the affected dimension**: look at the telemetry signal payload for `dimension_id`.
3. **Check the orchestrator queue depth**: high queue depth indicates backpressure.
   ```
   # Inspect TaskScheduler snapshot (CLI or widget)
   ify-ctl scheduler snapshot --dimension <dim-id>
   ```
4. **Check replication kernel health**: crash loops cause task starvation.
   ```
   ify-ctl kernel replica status --dimension <dim-id>
   ```

---

## Diagnosis

| Symptom | Likely Cause | Next Step |
|---------|-------------|-----------|
| p99 > 2000 ms, queue depth high | Orchestrator overloaded | Scale replicas or throttle submission rate |
| p99 high, queue empty | Slow task execution (CPU/IO bound) | Profile task execution; check resource caps |
| Availability < 99.9 % | Task failures / retry storms | Check error logs; inspect `RetryPolicy` |
| Intermittent spikes | Chaos fault injected | Disable chaos scenarios for this operation |

---

## Mitigation

1. **Reduce queue depth**: temporarily increase the `DimensionQuota` for the affected dimension.
2. **Restart stalled workers**: if replicas are stuck, trigger a crash-only restart.
3. **Disable problematic chaos scenario**: call `ChaosEngine::deactivate(scenario_id)`.
4. **Reduce retry storm**: lower `RetryPolicy::max_attempts` or increase `backoff_base_ms`.

---

## Recovery Verification

1. Confirm `task.p99_latency_ms` compliance ratio ≥ 99.9 % in the SLO panel.
2. Confirm `task.availability` passing in the SLO panel.
3. Resolve the incident in the `IncidentPipeline`.
4. Record consumed error budget seconds in `ReliabilityMetrics::task_error_budget`.

---

## Escalation

- If not resolved within **4 hours** (P0 SLA): page the platform-team on-call.
- If root cause is unknown after 1 hour: open a postmortem draft using `docs/reliability/postmortem-template.md`.

---

## References

- SLO definitions: [`ify-reliability::slo::SloRegistry::with_defaults`](../../runtime-rust/crates/ify-reliability/src/slo.rs)
- Task scheduler: [`ify-controller::scheduler`](../../runtime-rust/crates/ify-controller/src/scheduler.rs)
- Incident pipeline: [`ify-reliability::incident`](../../runtime-rust/crates/ify-reliability/src/incident.rs)
