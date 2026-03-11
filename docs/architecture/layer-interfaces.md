# Layer Interfaces — IDL/Specification for Cross-Layer APIs

**Status:** `[x]` complete  
**Epic:** L — Layered Module Interfaces  
**Owner:** Architecture team  
**Implemented in:** `runtime-rust/crates/ify-interfaces/`

---

## 1. Purpose

This document is the normative **Interface Definition Language (IDL)** for every
stable cross-layer API in infinityOS.  It defines the contract that all layers
must honour when communicating across layer boundaries.

All interfaces are published as Rust traits in the `ify-interfaces` crate.  The
crate depends, among infinityOS crates, only on `ify-core` (for primitive types)
so that every other layer can depend on it without creating circular dependencies.

### Dependency rule

```
ify-core  ←  ify-interfaces  ←  ify-controller (implements)
                              ←  ify-canvas     (consumes)
                              ←  ify-executor   (implements / consumes)
                              ←  ify-reliability (consumes)
```

No layer may implement or consume an interface that belongs to a layer above it
(see `AGENTS.md §8` rule 1).

---

## 2. Semver Contract

Every interface in this document is pinned to a version constant in
`ify-interfaces::versioning`.  The rules are defined in
[`docs/architecture/deprecation-policy.md`](deprecation-policy.md).

| Interface | Constant | Current version |
|-----------|----------|----------------|
| EventBusApi | `EVENT_BUS_API_VERSION` | `1.0.0` |
| OrchestratorBusApi | `EVENT_BUS_API_VERSION` | `1.0.0` |
| MeshArtifactApi | `MESH_ARTIFACT_API_VERSION` | `1.0.0` |
| MeshSubscriberApi | `MESH_ARTIFACT_API_VERSION` | `1.0.0` |
| NodePlannerApi | `NODE_EXECUTION_API_VERSION` | `1.0.0` |
| NodeExecutorApi | `NODE_EXECUTION_API_VERSION` | `1.0.0` |
| NodeReporterApi | `NODE_EXECUTION_API_VERSION` | `1.0.0` |
| EditorIntegrationApi | `EDITOR_INTEGRATION_API_VERSION` | `1.0.0` |

---

## 3. Event Bus API — ActionLog + Orchestration Events

### 3.1 `EventBusApi`

**Module:** `ify_interfaces::event_bus`  
**Reference implementation:** `ify_controller::action_log::ActionLog`

```
EventBusApi<Entry>
  append(entry: Entry)
  subscribe() → Receiver<Entry>
  entries_for_dimension(dim: DimensionId) → Vec<Entry>
  entries_for_task(task_id: TaskId) → Vec<Entry>
  all_entries() → Vec<Entry>
  len() → usize
  is_empty() → bool   [default: len() == 0]
```

**Guarantees:**
- `append` is infallible; all delivery errors are handled internally.
- `subscribe` returns a receiver that only sees entries appended *after* the call.
- Entries are stored in append-only order; no deletion or mutation is permitted.
- All reads are thread-safe.

### 3.2 `OrchestratorBusApi`

**Module:** `ify_interfaces::event_bus`  
**Reference implementation:** `ify_controller::orchestrator::LocalOrchestrator`

```
OrchestratorBusApi<Event, Error>
  submit(task_id, dimension_id, priority: u8, payload: Value) → Result<(), Error>
  progress(task_id, percent: u8, message: &str)              → Result<(), Error>
  complete(task_id)                                           → Result<(), Error>
  fail(task_id, error: &str)                                  → Result<(), Error>
  cancel(task_id)                                             → Result<(), Error>
  replay(task_id)                                             → Result<Vec<Event>, Error>
  subscribe()                                                 → Receiver<Event>
```

**Guarantees:**
- `submit` records the task in the orchestrator; duplicate `submit` calls for the
  same `task_id` overwrite the previous history (no idempotency guard in the
  current reference implementation).
- After any terminal event (`complete`, `fail`, `cancel`), subsequent calls
  return an error rather than silently mutating state.
- `replay` returns events in submission order.

### 3.3 Canonical event types

See `docs/architecture/event-taxonomy.md` for the full verb–noun taxonomy.
Every event emitted through the bus must use a registered `EventType` variant.

---

## 4. Mesh Artifact API — Read / Write / Subscribe

### 4.1 `MeshArtifactApi`

**Module:** `ify_interfaces::mesh`  
**Reference implementation:** `ify_controller::mesh::MeshArtifactStore`

```
MeshArtifactApi<Artifact, Snapshot, Patch, Error>
  produce(artifact: Artifact) → ArtifactId
  consume(id: ArtifactId)     → Result<Artifact, Error>
  snapshot_node(dimension_id, task_id, node_id, content: Value) → ArtifactId
  get_snapshot(id)                                               → Result<Snapshot, Error>
  patch(dimension_id, task_id, node_id, ops: Value)             → ArtifactId
  get_patch(id)                                                  → Result<Patch, Error>
  artifact_count()                                               → usize
```

**Immutability tiers** (defined in `ImmutabilityTier`):

| Value | Name | Lifetime |
|-------|------|---------|
| `Ephemeral` | 0 | Duration of producing task |
| `Session` | 1 | Duration of owning dimension |
| `Persistent` | 2 | Until explicitly archived or deleted |

**Guarantees:**
- `produce` assigns a globally unique, time-ordered `ArtifactId`.
- `consume` is a one-time read; subsequent calls return `Error::AlreadyConsumed`.
- Snapshots and patches are immutable once written.

### 4.2 `MeshSubscriberApi`

**Module:** `ify_interfaces::mesh`  
**Reference implementation:** `ify_controller::mesh::MeshArtifactStore`

```
MeshSubscriberApi
  subscribe() → Receiver<ArtifactId>
```

Receivers see the `ArtifactId` of every artifact written (produced, snapshotted,
or patched) *after* the subscription call.

---

## 5. Node Execution API — Planner → Executor → Reporter

The node execution lifecycle is split across three traits that correspond to the
three phases of a node run.

```text
NodePlannerApi                  NodeExecutorApi          NodeReporterApi
  plan(dimension_id) ──►plan──►  submit(task_id, ...)  ──► progress(task_id, %)
  validate(dimension_id)          cancel(task_id)           complete(task_id)
                                                            fail(task_id, msg)
                                                            cancel(task_id)
```

### 5.1 `NodePlannerApi`

**Module:** `ify_interfaces::node_execution`

```
NodePlannerApi<Plan, Error>
  plan(dimension_id) → Result<Plan, Error>
  validate(dimension_id) → Result<(), Vec<String>>
```

**Guarantees:**
- `plan` returns `Err` for graphs with cycles, unresolved ports, or validation failures.
- `validate` reports *all* issues (not just the first), enabling UI display of a full
  validation summary.

### 5.2 `NodeExecutorApi`

**Module:** `ify_interfaces::node_execution`  
**Reference implementation:** `ify_controller::orchestrator::LocalOrchestrator`

```
NodeExecutorApi<Error>
  submit(task_id, dimension_id, priority: u8, payload: Value) → Result<(), Error>
  cancel(task_id) → Result<(), Error>
```

### 5.3 `NodeReporterApi`

**Module:** `ify_interfaces::node_execution`  
**Reference implementation:** `ify_controller::orchestrator::LocalOrchestrator`

```
NodeReporterApi<Error>
  progress(task_id, percent: u8, message: &str) → Result<(), Error>
  complete(task_id)                              → Result<(), Error>
  fail(task_id, error_message: &str)             → Result<(), Error>
  cancel(task_id)                                → Result<(), Error>
```

**State machine:**

```
Idle ──submit──► Running ──progress*──► Complete
                        ╰──────────────► Failed
                        ╰──────────────► Cancelled
```

---

## 6. Editor Integration API — Interpreter Attach, LSP, Runtimes

### 6.1 `EditorIntegrationApi`

**Module:** `ify_interfaces::editor`  
**Reference implementation:** `ify_controller::registry::BlockRegistry`

```
EditorIntegrationApi<Error>
  register_block(dimension_id, task_id) → BlockId
  create_editor(block_id, language: &str) → Result<EditorRef, Error>
  attach_interpreter(block_id, interpreter_type: &str, config: Value)
                                          → Result<InterpreterRef, Error>
  bind_runtime(block_id)                 → Result<RuntimeHandle, Error>
  editor_for(block_id)                   → Option<EditorRef>
  binding_for(block_id)                  → Option<RuntimeHandle>
```

**Pipeline stages:**

```
register_block()
      │
      ▼
create_editor(block_id, language)        ← "rust" | "python" | "typescript" | ...
      │
      ▼
attach_interpreter(block_id, type, cfg)  ← "lsp" | "repl" | "jupyter" | ...
      │
      ▼
bind_runtime(block_id) ──► RuntimeHandle { executor_endpoint }
```

**Interpreter types (`interpreter_type`):**

| Value | Description |
|-------|-------------|
| `"lsp"` | Language Server Protocol (code intelligence, diagnostics) |
| `"repl"` | Interactive Read-Eval-Print Loop |
| `"jupyter"` | Jupyter kernel protocol |
| `"tree-sitter"` | Syntax-only parsing without LSP |

**Guarantees:**
- Stages must be executed in order; calling `bind_runtime` before `attach_interpreter`
  returns a `StageNotComplete` error.
- Each `block_id` can have at most one editor and one interpreter at a time.
- `RuntimeHandle.executor_endpoint` is a stable, opaque address for routing
  task submissions to the correct executor.

---

## 7. Compatibility Tests

Compatibility tests are located in `ify-controller/src/interfaces.rs`
under the `#[cfg(test)] mod conformance_tests` block.  Run with:

```sh
cargo test -p ify-controller interfaces
```

Tests cover:
- `EventBusApi` — append, query by dimension/task, subscribe
- `OrchestratorBusApi` — submit/progress/complete, fail/replay, subscribe
- `NodeExecutorApi` + `NodeReporterApi` — submit/progress/complete/cancel
- `MeshArtifactApi` — produce/consume/snapshot/patch
- `MeshSubscriberApi` — subscribe notifications
- `EditorIntegrationApi` — full pipeline, duplicate-editor rejection
- Version constants — all four API version constants accessible and at major 1

### Interface evolution compatibility test policy

When a new minor version of an interface is published:

1. Add a test that calls the new method and verifies the expected postcondition.
2. Add a test asserting `v_old.is_compatible_with(v_new)` returns `true`.
3. Add a test asserting `v_new.is_compatible_with(v_old)` returns `false`.

When a new major version is published (breaking change):

1. Add a test asserting `v_old.is_compatible_with(v_new)` returns `false`.
2. Add a `#[should_panic]` or `Result::Err` test documenting the removed behaviour.

---

## 8. Reference Implementations

| Interface | Reference implementation | Location |
|-----------|--------------------------|---------|
| `EventBusApi` | `ActionLog` | `ify-controller/src/action_log.rs` |
| `OrchestratorBusApi` | `LocalOrchestrator` | `ify-controller/src/orchestrator.rs` |
| `NodeExecutorApi` | `LocalOrchestrator` | `ify-controller/src/orchestrator.rs` |
| `NodeReporterApi` | `LocalOrchestrator` | `ify-controller/src/orchestrator.rs` |
| `MeshArtifactApi` | `MeshArtifactStore` | `ify-controller/src/mesh.rs` |
| `MeshSubscriberApi` | `MeshArtifactStore` | `ify-controller/src/mesh.rs` |
| `EditorIntegrationApi` | `BlockRegistry` | `ify-controller/src/registry.rs` |

---

## 9. Related Documents

- [`event-taxonomy.md`](event-taxonomy.md) — full verb–noun event taxonomy
- [`artifact-model.md`](artifact-model.md) — artifact classes and immutability tiers
- [`taskid-invariants.md`](taskid-invariants.md) — `TaskId` format and uniqueness guarantees
- [`dimension-model.md`](dimension-model.md) — dimension scoping model
- [`deprecation-policy.md`](deprecation-policy.md) — semver rules and migration process
- `docs/governance/interface-compatibility-policy.md` — review checklist
