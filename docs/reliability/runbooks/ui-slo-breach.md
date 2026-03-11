# Runbook: UI Responsiveness SLO Breach

**Epic K — Item 6**

**Applies to**: `ui.frame_time_ms`, `ui.interaction_latency_ms` SLOs

---

## Symptom

The reliability dashboard shows `ui.frame_time_ms` or `ui.interaction_latency_ms` in a **Degraded** state.  Users may report canvas lag or unresponsive controls.

---

## Immediate Actions (< 5 min)

1. **Check the `ui.frame_time_ms` SLO status** in the dashboard `slo_status` panel.
2. **Check the canvas `PerformanceBudget`** for the current zoom level.
3. **Enable adaptive culling** if not already active — reduces visible node count.
   ```rust
   // In the canvas performance monitor:
   let mut culler = AdaptiveCuller::new(budget.max_visible_nodes);
   culler.add_candidates(all_nodes.iter().cloned());
   let visible = culler.cull(&viewport_rect);
   ```
4. **Reduce zoom-level detail**: force a transition to `ZoomLevel::Overview` to reduce render load.

---

## Diagnosis

| Symptom | Likely Cause | Next Step |
|---------|-------------|-----------|
| Frame time > 16 ms (p99) | Too many visible nodes | Enable adaptive culling |
| Frame time high at Micro zoom | Complex node rendering | Reduce detail level at micro zoom |
| Interaction latency > 100 ms | Main-thread blocking | Move heavy work to background task |
| Intermittent jank | GC pause or lock contention | Profile canvas hot path |

---

## Mitigation

1. **Adaptive culling**: enforce `PerformanceBudget::max_visible_nodes` via `AdaptiveCuller`.
2. **Zoom level capping**: restrict maximum zoom to `ZoomLevel::Standard` during degraded mode.
3. **Defer non-critical updates**: batch canvas updates and apply at frame boundaries.
4. **Reduce collaboration sync rate**: lower cursor presence broadcast frequency.

---

## Recovery Verification

1. Confirm `ui.frame_time_ms` compliance ratio ≥ 99.9 % in the SLO panel.
2. Confirm `ui.interaction_latency_ms` passing in the SLO panel.
3. Resolve the incident in `IncidentPipeline`.
4. Record consumed error budget in `ReliabilityMetrics::ui_error_budget`.

---

## Escalation

- If not resolved within **24 hours** (P1 SLA): notify the canvas team lead.
- If frame times remain high after adaptive culling: open a postmortem.

---

## References

- SLO definitions: [`ify-reliability::slo::SloRegistry::with_defaults`](../../runtime-rust/crates/ify-reliability/src/slo.rs)
- Canvas performance: [`ify-canvas::performance`](../../runtime-rust/crates/ify-canvas/src/performance.rs)
- Postmortem template: [`docs/reliability/postmortem-template.md`](../postmortem-template.md)
