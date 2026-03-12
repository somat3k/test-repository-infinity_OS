# Quality Gates — Merge Readiness

**Status:** `[x]`
**Epic:** Q — Quality Engineering
**Owner:** Quality Engineering

---

## 1. Purpose

Define the set of non-negotiable quality gates that every merge request must
satisfy before it may be merged into any protected branch.  These gates are
enforced by CI and evaluated by `ify-quality::gates::QualityGateSet::canonical()`.

---

## 2. Gate Definitions

| Gate Name | Category | Metric | Operator | Threshold | Rationale |
|-----------|----------|--------|----------|-----------|-----------|
| `all-unit-tests-pass` | Test Passing | `unit_tests_passed` | == | 1.0 | All unit tests must pass |
| `all-integration-tests-pass` | Test Passing | `integration_tests_passed` | == | 1.0 | All integration tests must pass |
| `line-coverage-min-80` | Coverage | `line_coverage_pct` | ≥ | 80 % | Minimum line coverage |
| `branch-coverage-min-70` | Coverage | `branch_coverage_pct` | ≥ | 70 % | Minimum branch coverage |
| `no-critical-security-findings` | Security | `critical_security_findings` | ≤ | 0 | No critical vulnerabilities |
| `no-high-security-findings` | Security | `high_security_findings` | ≤ | 0 | No high vulnerabilities |
| `p99-latency-regression-max-20pct` | Performance | `p99_latency_regression_pct` | ≤ | 20 % | Latency regression budget |
| `throughput-regression-max-10pct` | Performance | `throughput_regression_pct` | ≤ | 10 % | Throughput regression budget |
| `contract-conformance-pass` | Contract Conformance | `contract_conformance_passed` | == | 1.0 | IDL contracts must pass |
| `changelog-present` | Documentation | `changelog_entry_present` | == | 1.0 | Changelog entry required |

---

## 3. CI Metric Production

CI must produce the following scalar metrics and write them into the
`MergeReadinessReport` before gates are evaluated:

| Metric | Produced by |
|--------|-------------|
| `unit_tests_passed` | `cargo test` / `ctest` exit code (1.0 = pass) |
| `integration_tests_passed` | Integration test job exit code |
| `line_coverage_pct` | `cargo llvm-cov` or `gcov` report |
| `branch_coverage_pct` | `cargo llvm-cov` or `gcov` report |
| `critical_security_findings` | SAST/SCA pipeline finding count |
| `high_security_findings` | SAST/SCA pipeline finding count |
| `p99_latency_regression_pct` | Benchmark comparison vs baseline JSON |
| `throughput_regression_pct` | Benchmark comparison vs baseline JSON |
| `contract_conformance_passed` | Contract test runner verdict |
| `changelog_entry_present` | Changelog linter (1.0 = present) |

---

## 4. Gate Evaluation

```rust
use ify_quality::gates::{QualityGateSet, MergeReadinessReport};

let gates = QualityGateSet::canonical();
let mut report = MergeReadinessReport::new();
// ... populate metrics from CI ...
let verdict = gates.evaluate_all(&report);
if !verdict.all_passed {
    for outcome in verdict.failed_gates() {
        eprintln!("GATE FAILED: {} (got {}, need {})",
            outcome.gate_name, outcome.metric_value, outcome.threshold);
    }
    std::process::exit(1);
}
```

---

## 5. Waiver Process

A gate may be waived only for a specific merge request if:

1. The waiver is approved by at least one `@codeowners` reviewer.
2. The waiver reason is documented in the ActionLog as `quality.gate.waived`.
3. A follow-up issue is opened to restore compliance within 2 sprint cycles.

---

## 6. References

- [`test-strategy.md`](test-strategy.md)
- [`release-candidate-checklist.md`](release-candidate-checklist.md)
- [`docs/governance/release-gates.md`](../governance/release-gates.md)
- `ify-quality::gates` module
