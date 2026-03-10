# tests/unit — Unit Tests

Unit tests for individual functions and modules within each layer.

## Organization

Unit tests that cannot be co-located with source code (e.g., C kernel tests) live here. Rust crate tests live in their respective `src/lib.rs` files under `#[cfg(test)]` modules.

## Conventions

- One test file per module under test.
- Tests must be deterministic and hermetic (no network, no real filesystem).
- Test names follow the pattern `<module>_<scenario>_<expected_outcome>`.
- Every test must complete in < 100 ms.
