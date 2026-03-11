# Runbook: Replication Kernel Replica Crash

**Epic K — Item 6**

**Applies to**: `replication.*` operations; surfaced by `ChaosEngine` `ReplicaCrash` faults and real kernel replica crashes.

---

## Symptom

- A replica ID appears as unavailable in the kernel service registry.
- Tasks routed to the crashed replica time out or fail with `ResourceExhausted`.
- The dashboard `incidents` panel shows an open incident with signal `chaos.fault_injected` (for injected faults) or a kernel trace error (for real crashes).

---

## Immediate Actions (< 5 min)

1. **Identify the crashed replica**:
   ```
   ify-ctl kernel replica status --all
   ```
2. **Check the restart policy**: the service registry applies exponential backoff (base 100 ms, cap 30 s).  Verify the replica is cycling through restarts.
3. **Check resource caps**: replica may have exceeded memory or CPU limits.
4. **Reroute in-flight tasks**: the orchestrator's retry policy will automatically retry failed tasks on a healthy replica after `backoff_base_ms` delay.

---

## Diagnosis

| Symptom | Likely Cause | Next Step |
|---------|-------------|-----------|
| Replica crash loop (> 3 restarts) | Config error or resource exhaustion | Inspect kernel trace logs; check `ReplicaPolicy` caps |
| Single crash, clean restart | Transient fault | Monitor; verify tasks complete on retry |
| Multiple replicas crashing | Systemic issue (memory pressure, bad workload) | Reduce replica count; increase resource caps |
| Chaos fault `ReplicaCrash` active | Intentional chaos test | Deactivate chaos scenario if not a drill |

---

## Mitigation

1. **Increase resource caps** if the replica was OOM-killed:
   ```rust
   // Adjust ReplicaPolicy in replication.c / Rust FFI layer
   policy.max_memory_mb = 512;
   ```
2. **Pause new task submissions** to the affected dimension while replicas recover.
3. **Deactivate chaos scenario** if this is an unintended fault injection:
   ```rust
   engine.deactivate("replica-crash-scenario")?;
   ```
4. **Force a clean restart** if the replica is stuck:
   ```
   ify-ctl kernel replica restart --id <replica-id>
   ```

---

## Recovery Verification

1. Confirm replica appears as `healthy` in `ify-ctl kernel replica status`.
2. Confirm pending tasks have been retried and completed.
3. Confirm `task.availability` SLO is passing.
4. Resolve the incident in `IncidentPipeline`.

---

## Escalation

- If replicas continue crashing after 3 restart cycles: escalate to kernel team.
- If memory caps cannot be increased: reduce dimension concurrency quota.

---

## References

- Replication kernel: [`kernel-c/src/replication.c`](../../../kernel-c/src/replication.c)
- Service registry restart policy: [`kernel-c/src/service_registry.c`](../../../kernel-c/src/service_registry.c)
- Chaos engine: [`ify-reliability::chaos`](../../runtime-rust/crates/ify-reliability/src/chaos.rs)
- Task scheduler retry policy: [`ify-controller::scheduler::RetryPolicy`](../../runtime-rust/crates/ify-controller/src/scheduler.rs)
