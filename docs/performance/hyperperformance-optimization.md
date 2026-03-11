# Epic H — Hyperperformance Optimization

This document defines the baseline budgets, instrumentation hooks, and optimization policies that finalize Epic H.
It codifies the expectations used by the Reliability and Operational agents when measuring system performance.

## Baseline Performance Budgets

These budgets define the initial targets per subsystem. They serve as guardrails and can be tightened once measured baselines improve.

| Subsystem | Latency Target (p99) | Throughput Target | Memory Budget | Notes |
| --- | --- | --- | --- | --- |
| Kernel (C) | < 2 ms scheduler tick | ≥ 250k task ops/sec | ≤ 64 MB overhead | Focus on scheduler, memory allocator, FFI. |
| Performer Runtime (Rust) | < 20 ms task dispatch | ≥ 25k tasks/sec | ≤ 1.5 GB per dimension | Includes ActionLog and mesh publish. |
| Canvas | < 16 ms input-to-paint | 60 FPS sustained | ≤ 1 GB for 10k-node graph | Performance budget for UX interactions. |
| Data | < 200 ms query p99 | ≥ 500 MB/s ingest | ≤ 4 GB per dataset worker | Benchmarked per adapter. |
| Deploy | < 5 s rollout update | ≥ 1k deploy events/min | ≤ 512 MB control-plane | Includes canary and rollback decisions. |
| Agents | < 2 s plan update | ≥ 200 actions/min | ≤ 512 MB per agent | Focus on ActionLog and mesh write latency. |

## Profiling Hooks (Critical Paths)

Instrumentation must be present in the following paths:

- Scheduler enqueue/dequeue, context switches, and wake-up delays.
- ActionLog append/subscribe, mesh artifact publish/consume, and task dispatch.
- Node execution lifecycle: start/progress/complete/fail/cancel.
- Canvas render loop and graph diff/patch application.

Every profiling record must include: `dimension_id`, `task_id`, `action_id`, `correlation_id`, and wall-clock duration.
Agents with the `CAP_PERF` capability may request extended counters (CPU cycles, cache misses).

## Benchmark Suite

The benchmark suite is owned by the Reliability agent and follows the conventions in `tests/perf/README.md`.

- **Kernel**: build `bench` target via `cmake --build build --target bench`.
- **Runtime**: `cargo bench` benchmarks ActionLog, mesh, task dispatch, and flow control.
- **Canvas/Graph**: use a deterministic replay harness that loads a 10k-node graph and replays 1k edits.
- **Data**: measure ingest/query/restore baselines for each adapter.

Benchmarks must emit throughput, p50/p95/p99 latency, and maximum values.

## Load Testing (Mesh + Batching)

Load tests stress mesh artifact updates and adaptive batching thresholds:

- Simulate 10k artifact updates/min across 50 dimensions.
- Use staggered node batch sizes (16/32/64) to validate backpressure signals.
- Capture queue depth and drop rates; no loss is permitted without ActionLog entries.

## Hot Path Optimization Targets

- Minimize scheduler lock contention with per-dimension queues and batch dequeues.
- Use preallocated buffers for ActionLog and mesh writes where possible.
- Avoid heap churn in high-frequency loops; prefer arena-backed allocators.

## Serialization and Zero-Copy Strategy

- Prefer `bytes::Bytes`-backed payloads for mesh artifacts and ActionLog payloads.
- Avoid intermediate JSON allocations on hot paths; use binary encodings for internal hops.
- Maintain a compatibility layer for external integrations that require JSON.

## Caching Strategy

- **Artifact cache**: LRU per dimension, size capped at 256 MB with TTL by immutability tier.
- **Node results cache**: keyed by TaskID + node hash, invalidated on schema changes.
- **Agent planning cache**: cache resolved capability sets for 24h unless a policy update occurs.

## Adaptive Batching Policies

Batching adapts to backpressure and queue depth:

- Target batch sizes of 16–64 tasks, with a 50 ms max wait before flush.
- Reduce batch size by half when queue depth exceeds 2× budget.
- Emit ActionLog entries when batches are throttled or dropped.

## Optimization Loops and Measurable Gains

The Reliability agent runs a weekly Kaizen review:

1. Capture baseline metrics and compare against budgets.
2. Identify top 3 regressions or slow paths.
3. Propose optimizations with expected gains.
4. Validate improvements with benchmarks and load tests.
5. Record measured gains in the optimization log.

## Regression Guardrails

Use the performance regression thresholds defined in `tests/perf/README.md`:

- **Throughput regression**: > 10% drop from baseline.
- **Latency regression**: > 20% increase in p99.

Any regression must fail CI and trigger a Reliability agent incident report.
