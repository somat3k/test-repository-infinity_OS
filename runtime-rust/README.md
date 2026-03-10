# runtime-rust — Rust Performer Runtime

The Rust performer runtime provides safe, concurrent orchestration over the C kernel. It owns the agent task lifecycle, node graph execution, deployment adapters, and the typed contracts between all upper layers.

## Responsibilities

- Task execution engine with priority scheduling, retries, and cancellation
- Safe FFI boundary wrappers around C kernel calls
- Shared type definitions: `TaskId`, `DimensionId`, `ArtifactId`, capability flags
- Planner integration: plans → tasks → nodes
- Memory subsystem hooks (short/long-term, vector store)
- Structured logging and distributed tracing

## Crates

| Crate | Purpose |
|-------|---------|
| `ify-core` | Shared types, identifiers, capability flags, error kinds |
| `ify-executor` | Task executor: lifecycle, scheduling, cancellation, retries |
| `ify-ffi` | Safe Rust wrappers over the C kernel ABI |

## Build

```sh
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## Constraints

- **Safe concurrency**: all shared state is protected by `Arc`/`Mutex` or channel-based patterns.
- **Typed contracts**: every cross-module boundary uses explicit Rust types, not raw pointers.
- **Recoverable failures**: every error path returns `Result<T, E>` with context; panics are forbidden in library code.
- **FFI safety**: all calls into the C kernel go through `ify-ffi` with explicit lifetime and alignment guarantees.

## Epic Tracking

See [EPIC R — Rust Performer Runtime](../TODO.md) in `TODO.md`.
