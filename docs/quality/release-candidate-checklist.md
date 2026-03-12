# Release Candidate Validation Checklist

**Status:** `[x]`
**Epic:** Q — Quality Engineering
**Owner:** Release Engineering

---

## 1. Purpose

Define the structured checklist that must be completed and signed off before
any kernel, runtime, or cross-layer release is tagged.  This checklist is
instantiated per RC via `ify-quality::rc_checklist::RcChecklist::canonical_template()`,
persisted as a mesh artifact, and exported as a Markdown report.

---

## 2. Checklist Template

Items are grouped by category.  `[blocking]` items must be resolved (complete,
signed-off, or waived) before the release tag may be created.

### Testing

| ID | Blocking | Description | Evidence Required |
|----|----------|-------------|-------------------|
| `unit-tests-pass` | ✅ | All unit test suites pass with no failures | CI test job URL or cargo test output |
| `integration-tests-pass` | ✅ | All integration test suites pass with no failures | CI integration job URL |
| `coverage-thresholds-met` | ✅ | Line ≥ 80 % and branch ≥ 70 % for all layers | Coverage report artifact link |
| `contract-conformance-verified` | ✅ | All IDL contract conformance probes pass | Contract test run report |
| `fuzz-campaigns-clean` | ⚪ | No new crashes since previous RC | Fuzz campaign summary artifact |

### Performance

| ID | Blocking | Description | Evidence Required |
|----|----------|-------------|-------------------|
| `perf-no-throughput-regression` | ✅ | Orchestrator and mesh throughput within 10 % of baseline | Benchmark comparison report |
| `perf-no-p99-regression` | ✅ | p99 latency within 20 % of baseline for all measured paths | Benchmark comparison report |

### Security

| ID | Blocking | Description | Evidence Required |
|----|----------|-------------|-------------------|
| `sast-scan-clean` | ✅ | SAST pipeline reports no critical/high findings | SAST scan report artifact |
| `dast-scan-clean` | ✅ | DAST pipeline reports no high-risk findings | DAST scan report artifact |
| `sbom-generated` | ⚪ | SBOM generated and attached as release artifact | SBOM artifact link |

### Documentation

| ID | Blocking | Description | Evidence Required |
|----|----------|-------------|-------------------|
| `changelog-updated` | ✅ | CHANGELOG entry written for this release | CHANGELOG.md diff link |
| `api-docs-regenerated` | ⚪ | Rust API docs regenerated with `cargo doc` | cargo doc output / docs.rs preview |

### Compatibility

| ID | Blocking | Description | Evidence Required |
|----|----------|-------------|-------------------|
| `abi-compatibility-confirmed` | ✅ | Kernel ABI version negotiation tested | ABI conformance test run output |

### Operations

| ID | Blocking | Description | Evidence Required |
|----|----------|-------------|-------------------|
| `runbook-reviewed` | ✅ | Operational runbook reviewed and updated | Runbook diff link |

_Legend: ✅ blocking — release cannot proceed without resolution. ⚪ advisory — encouraged but not blocking._

---

## 3. Completion States

| State | Meaning |
|-------|---------|
| `pending` | Not yet started |
| `complete` | Done but not yet reviewed |
| `signed_off` | Reviewed and approved by a named reviewer |
| `waived` | Explicitly waived with documented reason |

---

## 4. Usage

```rust
use ify_quality::rc_checklist::RcChecklist;

// Create a checklist for an RC.
let mut cl = RcChecklist::canonical_template("v0.2.0-rc1", "2026-03-12T00:00:00Z");

// Mark items complete as evidence is collected.
cl.item_mut("unit-tests-pass")
    .unwrap()
    .mark_complete("https://ci.example.com/job/456");

// Sign off once reviewed.
cl.item_mut("unit-tests-pass")
    .unwrap()
    .sign_off("release-eng", "2026-03-12T10:00:00Z")
    .unwrap();

// Export as Markdown.
println!("{}", cl.to_markdown());

// Check release readiness: all blocking items must be resolved.
assert!(cl.is_release_ready());
```

---

## 5. Release Readiness Gate

The CI release job evaluates `checklist.is_release_ready()` and fails if any
blocking item is unresolved.  The checklist is persisted to the mesh as:

```
mesh://release/<rc_id>/checklist.json
```

---

## 6. References

- [`test-strategy.md`](test-strategy.md)
- [`quality-gates.md`](quality-gates.md)
- [`docs/governance/release-gates.md`](../governance/release-gates.md)
- `ify-quality::rc_checklist` module
