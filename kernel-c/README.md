# kernel-c — C Kernel and Boost Layer

The C kernel provides the lowest-level primitives for infinityOS: memory management, scheduling, capability discovery, and the ABI-stable surface consumed by the Rust Performer Runtime.

## Responsibilities

- Deterministic memory allocation and reclamation (arenas, refcount/RCU)
- Cooperative and preemptive scheduler primitives (queues, priorities, timers)
- Capability discovery (hardware features, sandbox permissions)
- Crash-only restart semantics with state-recovery policy
- ABI-stable FFI export surface for Rust integration

## Constraints

- **No upward dependencies**: the kernel must not reference canvas, data, agents, or deploy layers.
- **Warnings-as-errors**: all modules compile with `-Wall -Wextra -Werror`.
- **Bounds checks mandatory**: every buffer access is validated at entry points.
- **ABI stability**: exported types in `include/infinity/ffi.h` are versioned and never changed without a major version bump.

## Directory Layout

```
include/infinity/    Public ABI headers (stable surface)
  kernel.h           Core types, version macros, lifecycle API
  memory.h           Memory allocator and arena interface
  scheduler.h        Scheduler queues, priorities, timer interface
  ffi.h              ABI-stable export surface for Rust FFI
src/                 Kernel implementation (not yet scaffolded)
tests/               Kernel unit and interface tests (not yet scaffolded)
CMakeLists.txt       Build configuration (to be added)
```

## Build

```sh
cmake -B build -DCMAKE_BUILD_TYPE=Debug   # or Release / Perf
cmake --build build
ctest --test-dir build --output-on-failure
```

## Epic Tracking

See [EPIC C — C Kernel and Boost Layer](../TODO.md) in `TODO.md`.
