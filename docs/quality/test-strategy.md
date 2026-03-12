# Test Strategy — infinityOS Quality Engineering

**Status:** `[x]`
**Epic:** Q — Quality Engineering
**Owner:** Quality Engineering

---

## 1. Purpose

Define a unified, pyramid-based test strategy that applies consistently across
the infinityOS kernel (C), Rust runtime, canvas, reliability, and security layers.

---

## 2. Test Pyramid

```
          /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
         /   Performance Tests  \        (cargo bench / cmake bench — nightly only)
        /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
       /   Integration Tests       \     (cargo test / ctest — every merge request)
      /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
     /       Unit Tests               \  (cargo test / ctest — every commit)
    /‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾‾\
```

### 2.1 Unit Tests

- **Scope:** individual functions, modules, and structs — no cross-crate calls.
- **Location:** Rust tests live in `#[cfg(test)]` modules inside each source file; C kernel tests live in `kernel-c/tests/`.
- **Budget:** ≤ 100 ms per test, ≤ 60 s total suite.
- **Hermeticity:** no real network, no real filesystem, no real time.
- **Naming:** `<module>_<scenario>_<expected_outcome>`.
- **Required coverage:** ≥ 80 % line, ≥ 70 % branch per layer.
- **Gate:** required to pass before any merge request is merged.

### 2.2 Integration Tests

- **Scope:** cross-layer flows — C ↔ Rust FFI, ActionLog completeness, mesh provenance chains.
- **Location:** `tests/integration/`.
- **Budget:** ≤ 10 s per test, ≤ 300 s total suite.
- **Allowed I/O:** real file I/O in a temp directory (cleaned up on completion). No external network.
- **Documentation:** each scenario must reference the applicable flow in `docs/architecture/reference-flow.md`.
- **Required coverage:** ≥ 70 % line, ≥ 60 % branch for cross-layer paths.
- **Gate:** required to pass before any merge request is merged.

### 2.3 Performance Tests

- **Scope:** throughput, latency (p50/p95/p99/max), and memory footprint baselines.
- **Location:** `tests/perf/`; Rust benchmarks use `cargo bench`; C benchmarks use CMake bench target.
- **Baselines:** checked in as `tests/perf/baselines/<name>.json`.
- **Regression definition:** > 10 % throughput degradation or > 20 % p99 latency increase vs baseline.
- **Gate:** not required for every merge request but must pass before release tagging.

---

## 3. Additional Test Types

| Type | Tool / Location | Gate |
|------|-----------------|------|
| Fuzz | `runtime-rust/fuzz/` (cargo-fuzz) | No new crashes |
| SAST | cargo-audit, cargo-deny, Semgrep, CodeQL | No critical/high findings |
| DAST | OWASP ZAP | No high findings against local API |
| Contract | `ify-quality::contract` | All mandatory invariants pass |
| Golden (UI) | `tests/fixtures/golden/`, `ify-quality::golden` | No snapshot diffs |
| Load | `ify-quality::load` | Within throughput and latency budgets |

---

## 4. Coverage Requirements

| Layer | Line Coverage | Branch Coverage |
|-------|---------------|-----------------|
| Kernel (C) | ≥ 80 % | ≥ 70 % |
| Runtime Core | ≥ 80 % | ≥ 70 % |
| Controller | ≥ 80 % | ≥ 70 % |
| Canvas | ≥ 80 % | ≥ 70 % |
| Reliability | ≥ 80 % | ≥ 70 % |
| Security | ≥ 80 % | ≥ 70 % |
| Cross-layer | ≥ 70 % | ≥ 60 % |

---

## 5. Determinism Requirements

- Tests must not depend on wall-clock time, random seeds (unless explicitly seeded), or external services.
- Deterministic test datasets are provided by `ify-quality::datasets` (see also `tests/fixtures/`).
- Test output is structured (JSON / SARIF) for machine-readable reporting via `ify-quality::report`.

---

## 6. Running the Suites

```sh
# Unit + integration (Rust)
cd runtime-rust
cargo test

# Unit + integration (C kernel)
cd kernel-c
cmake -B build && cmake --build build && ctest --test-dir build

# Performance (Rust)
cd runtime-rust
cargo bench

# Performance (C kernel)
cd kernel-c
cmake --build build --target bench
```

---

## 7. References

- [`quality-gates.md`](quality-gates.md)
- [`release-candidate-checklist.md`](release-candidate-checklist.md)
- [`docs/governance/release-gates.md`](../governance/release-gates.md)
- `ify-quality` crate: `runtime-rust/crates/ify-quality/`
