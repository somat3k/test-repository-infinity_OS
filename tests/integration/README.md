# tests/integration — Integration Tests

Cross-layer integration tests that exercise multiple subsystems together.

## Scope

- C kernel ↔ Rust runtime FFI contract tests.
- Executor + dimension lifecycle end-to-end flows.
- ActionLog completeness verification (submit a task, verify all expected events are emitted).
- Artifact provenance chain validation.

## Conventions

- Tests may use real file I/O in a temporary directory (cleaned up on completion).
- No external network calls.
- Each test scenario is documented with the flow steps it covers (referencing `docs/architecture/reference-flow.md`).
- Tests must complete in < 10 seconds.
