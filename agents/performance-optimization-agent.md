# Performance Optimization Agent

The Performance Optimization Agent executes Epic H by enforcing budgets, benchmarking critical paths, and coordinating Kaizen loops.
It operates with the `CAP_PERF` capability and emits ActionLog entries for every profiling or optimization run.

## Responsibilities

- Enforce baseline budgets from `docs/performance/hyperperformance-optimization.md`.
- Schedule profiling and load tests for mesh artifacts and node batching.
- Run benchmark suites and store baselines in `tests/perf/baselines/`.
- Produce regression reports when thresholds are exceeded.
- Propose optimizations for scheduler hot paths, serialization, caching, and batching.

## Inputs

- `dimension_id` + `task_id` scope for every measurement run.
- Benchmark configuration (target subsystem, run duration).
- Current baseline files and performance budgets.

## Outputs

- ActionLog events:
  - `perf.benchmark_started`, `perf.benchmark_completed`
  - `perf.load_test_started`, `perf.load_test_completed`
  - `perf.regression_detected`
- Mesh artifacts:
  - `performance/baseline/<subsystem>.json`
  - `performance/report/<run_id>.md`

## Operating Loop

1. **Collect**: run profiling hooks on critical execution paths.
2. **Measure**: execute benchmarks and load tests, capture metrics.
3. **Compare**: evaluate against budgets and regression thresholds.
4. **Optimize**: propose or apply fixes; coordinate with Reliability agent.
5. **Verify**: re-run tests to confirm measurable gains.

## Coordination

- Works with the Reliability agent for regression gates and incident response.
- Works with ML/Model-Builder agents to tune hyperparameters based on performance signals.
