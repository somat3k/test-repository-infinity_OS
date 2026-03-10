# kernel-c — C Kernel and Boost Layer

The C kernel provides the lowest-level primitives for infinityOS: memory management, scheduling, capability discovery, and the ABI-stable surface consumed by the Rust Performer Runtime.

## Responsibilities

- Deterministic memory allocation and reclamation (arenas, refcount/RCU)
- Cooperative and preemptive scheduler primitives (queues, priorities, timers)
- Capability discovery (hardware features, sandbox permissions)
- Crash-only restart semantics with state-recovery policy
- ABI-stable FFI export surface for Rust integration
- Kernel service registry (named services, lifecycle, health checks)
- Replication kernel (task-scoped micro-kernel instances, replication policies)
- Kernel tracing hooks (span IDs, timestamps, alloc stats) feeding telemetry

## Constraints

- **No upward dependencies**: the kernel must not reference canvas, data, agents, or deploy layers.
- **Warnings-as-errors**: all modules compile with `-Wall -Wextra -Werror`.
- **Bounds checks mandatory**: every buffer access is validated at entry points.
- **ABI stability**: exported types in `include/infinity/ffi.h` are versioned and never changed without a major version bump.

## Directory Layout

```
include/infinity/        Public ABI headers (stable surface)
  kernel.h               Core types, version macros, lifecycle API
  memory.h               Memory allocator and arena interface
  scheduler.h            Scheduler queues, priorities, timer interface
  ffi.h                  ABI-stable export surface for Rust FFI
  service_registry.h     Named-service registry with crash-only restart
  replication.h          Replication kernel (task-scoped micro-kernels)
  trace.h                Tracing hooks and span-based telemetry
src/                     Kernel implementation
  internal.h             Internal shared types (not public ABI)
  kernel.c               Boot sequence, lifecycle, capability discovery
  memory.c               General-purpose allocator and arena implementation
  scheduler.c            Priority queues, timers, cooperative scheduling
  ffi.c                  ABI negotiation, dimension + TaskID management
  service_registry.c     Service registry + crash-only restart
  replication.c          Replication kernel creation and management
  trace.c                Ring-buffer span collector and emit callbacks
tests/                   Kernel unit tests (7 test programs, 100% pass)
  harness.h              Minimal assert-based test harness
  test_kernel.c          Lifecycle, version, capabilities
  test_memory.c          Allocator and arena tests
  test_scheduler.c       Queue, priority, timer tests
  test_ffi.c             ABI info, dimension, TaskID tests
  test_service_registry.c  Service lifecycle and crash-restart tests
  test_replication.c     Replica create/submit/destroy tests
  test_trace.c           Span begin/end, ring buffer, emit callback tests
CMakeLists.txt           Build configuration (C11, -Wall -Wextra -Werror)
```

## Build

```sh
cmake -B build -DCMAKE_BUILD_TYPE=Debug   # or Release / Perf
cmake --build build
ctest --test-dir build --output-on-failure
```

All 7 test programs pass.

## Epic Tracking

See [EPIC C — C Kernel and Boost Layer](../TODO.md) in `TODO.md`.  All 10 Epic C items are complete as of 2026-03-10.
