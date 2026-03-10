# infinityOS

> **An operational system built around an infinity zoom canvas where code is infrastructure.**

infinityOS is a canvas-first development and execution platform. Nodes on the canvas represent code, data pipelines, agents, and deployment units that connect, group into instances, and are coordinated by dimensional `blockControllerGenerator` regimes—enabling Kaizen-style continuous optimization at every layer.

---

## Architecture Overview

infinityOS is implemented in two primary languages with strict separation of concerns:

| Layer | Language | Purpose |
|-------|----------|---------|
| **Kernel** | C (C17) | Memory, scheduling, system-facing primitives, ABI-stable surface |
| **Performer Runtime** | Rust | Orchestration, task lifecycle, agent flows, safe FFI over Kernel |
| **Canvas** | TBD | Node graph, mesh data, infinity zoom UX contracts |
| **Data** | TBD | Archival, storage, transform pipelines, lineage |
| **Agents** | TBD | Built-in agent templates, policies, execution flows |
| **Deploy** | TBD | Deployment adapters, workload targets, autoscaling |

Dependencies flow **downward only**. No layer may depend on a layer above it. See [`AGENTS.md`](AGENTS.md) for full agent roles and operating rules, and [`TODO.md`](TODO.md) for the A–Z epic roadmap.

```
User ──► Canvas ──► Performer Runtime ──► Kernel (C)
          ▲               ▲
         Data           Agents
          ▲               ▲
        Deploy ──────────►┘
```

## Repository Structure

```
kernel-c/              C kernel + boost layer
  include/infinity/    Public ABI headers
  src/                 Kernel implementation
  tests/               Kernel unit tests
runtime-rust/          Rust performer runtime
  crates/
    ify-core/          Shared types: TaskId, DimensionId, ArtifactId, capabilities
    ify-executor/      Task execution engine
    ify-ffi/           Safe FFI bindings over the C kernel
canvas/                Node graph + mesh canvas logic
data/                  Archival, storage, transform pipelines
agents/                Built-in agent templates and policies
deploy/                Deployment adapters and manifests
tests/
  unit/                Language-specific unit tests
  integration/         Cross-layer integration tests
  perf/                Performance and load tests
docs/
  architecture/        Epic A specification documents
AGENTS.md              Agent roles, build workflow, operational rules
TODO.md                A–Z epic roadmap with status tracking
```

## Quick Start

> **Build tooling is not yet checked in.** The kernel and runtime layer stubs are in place. Follow the target workflow in [`AGENTS.md §4`](AGENTS.md) once toolchain files are added.

### Kernel (C)

```sh
# From kernel-c/
cmake -B build -DCMAKE_BUILD_TYPE=Debug
cmake --build build
ctest --test-dir build
```

### Performer Runtime (Rust)

```sh
# From runtime-rust/
cargo build
cargo test
cargo clippy -- -D warnings
```

## Operational Rules

1. **Contract-first**: define/adjust interfaces before implementation.
2. **No silent failures**: every runtime failure path returns actionable context.
3. **Performance is a feature**: benchmark critical paths for each significant change.
4. **Secure by default**: validate all external inputs at layer boundaries.
5. **Version everything**: graph schemas, APIs, data transforms.
6. **Reproducible builds**: pin toolchains when build systems are introduced.
7. **Kaizen loop**: each sprint must include at least one measurable reliability or throughput improvement.

## Contributing

See [`AGENTS.md`](AGENTS.md) for agent roles and responsibilities. All cross-layer interface changes must be documented in `docs/architecture/` before implementation and tracked in [`TODO.md`](TODO.md).

## License

See [`LICENSE`](LICENSE).