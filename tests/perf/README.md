# tests/perf — Performance Tests

Performance and load tests that establish baselines and detect regressions.

## Scope

- Scheduler throughput: tasks submitted/second at various concurrency levels.
- Memory allocator throughput: arena alloc/reset cycles.
- Executor latency: p50/p95/p99 task dispatch latency.
- ActionLog write throughput: events/second sustained.
- Artifact write throughput: MB/s for various immutability tiers.

## Conventions

- Each benchmark reports: throughput, p50, p95, p99, max.
- Baselines are checked in as `baselines/<name>.json` and compared in CI.
- A regression is defined as a > 10% degradation in throughput or > 20% increase in p99 latency.
- Perf tests are not run in the default `cargo test` pass; use `cargo bench` or the dedicated CI job.

## Running

```sh
# From runtime-rust/
cargo bench

# From kernel-c/
cmake --build build --target bench
```
