# Block Controller Contract

**Status:** `[x]` complete  
**Epic:** B — blockControllerGenerator Regime  
**Owner:** Performer Agent  
**Implemented in:** `runtime-rust/crates/ify-controller`

---

## 1. Purpose

The **blockControllerGenerator regime** is the coordination layer between the
canvas editor, the interpreter runtime, the executor, and the mesh artifact
store.  Every canvas block (node) that the user creates or the agent plans is
managed by a `BlockController` that lives for exactly the duration of that
block's lifetime.

---

## 2. Dimensional Scoping

Every `BlockController` is bound to exactly one **dimension** at construction
time.  The `dimension_id` is:

- Set at `create()` and **immutable** for the controller's entire lifetime.
- Used as the scope key for all ActionLog entries emitted by the controller.
- Validated against any peer dimension supplied to `link()` — a mismatch
  returns `BlockControllerError::DimensionMismatch`.

Cross-dimension wiring is permitted (a controller in dimension A may link to a
peer node in dimension B), but the validation step must be explicitly waived by
the caller; it is not bypassed automatically.

---

## 3. Controller Lifecycle

```
create(dimension_id, task_id)
     │
     ▼
 ┌─────────┐
 │ Created │◄──────────────────────────── initial state
 └────┬────┘
      │ link(peer_dimension)
      ▼
 ┌────────┐
 │ Linked │◄──────────────────────────── peer dimension recorded
 └────┬───┘
      │ isolate()
      ▼
 ┌──────────┐
 │ Isolated │◄──────────────────────────── severed from peer
 └─────┬────┘
       │ dispose()
       ▼
 ┌──────────┐
 │ Disposed │◄──────────────────────────── resources freed
 └──────────┘
```

`dispose()` is also legal directly from `Created` or `Isolated` for emergency
cleanup without completing the full lifecycle.

### Invariants

1. Each `BlockController` has a globally unique `id` (UUID v4).
2. `dimension_id` is immutable after `create()`.
3. State transitions are monotonically forward-only; no state can be re-entered.
4. Every transition emits an [`ActionLogEntry`](event-taxonomy.md) before
   returning.
5. A controller that has been `dispose()`d rejects all further calls with
   `BlockControllerError::AlreadyDisposed`.

---

## 4. Block Registration Pipeline

Before a block can execute tasks it must pass through four pipeline stages.
Each stage is guarded by the preceding stage's completion.

| Stage | Method | Emits |
|-------|--------|-------|
| 1 | `BlockRegistry::register_block(dim, task_id)` | `controller.registered` |
| 2 | `create_editor(block_id, language)` | `editor.created` |
| 3 | `attach_interpreter(block_id, type, config)` | `interpreter.attached` |
| 4 | `bind_runtime(block_id)` | `runtime.bound` |

Calling a stage out of order returns `RegistryError::StageNotComplete`.

---

## 5. TaskID Allocator

The `TaskAllocator` provides two allocation modes:

### 5.1 Monotonic (UUID v7)

```rust
let task_id = allocator.next(dimension_id)?;
```

- Requires `register_dimension(dim)` before first use.
- IDs are UUID v7 (millisecond-resolution timestamp + random bits), guaranteeing
  non-decreasing ordering within a single process clock window.
- IDs are **globally unique** across all dimensions and processes (via the
  random component).

### 5.2 Deterministic (UUID v5)

```rust
let task_id = allocator.derive(dimension_id, "workflow-name");
```

- Produces the same `TaskId` for the same `(dimension_id, name)` pair on every
  call, regardless of time or process restart.
- Uses UUID v5 (SHA-1) with a fixed application namespace
  (`7a9f3c1e-8b4d-5e2f-a601-3c7d9e0b1f24`).
- Useful for idempotent operations: the caller can re-derive the ID without
  shared state.

---

## 6. ActionLog Contract

Every controller action **MUST** emit an `ActionLogEntry` before returning.
The minimum required fields are:

| Field | Requirement |
|-------|-------------|
| `event_id` | Fresh `ArtifactId` (UUID v7) |
| `event_type` | Verb-noun string from the [event taxonomy](event-taxonomy.md) |
| `occurred_at_ms` | Unix epoch milliseconds (monotonic wall clock) |
| `dimension_id` | The controller's dimension (or `None` for kernel events) |
| `task_id` | The owning task (`None` if not task-scoped) |
| `actor` | `Actor::System` for automatic actions; `Actor::User` or `Actor::Agent` for human-triggered ones |
| `payload` | Verb-specific JSON fields (see §3.6 of the taxonomy) |

Optional causality/correlation fields must be propagated from the triggering
event where available.

### Controller-specific event types

| Event | Trigger |
|-------|---------|
| `controller.registered` | `BlockController::create()` or `BlockRegistry::register_block()` |
| `controller.linked` | `BlockController::link()` |
| `controller.isolated` | `BlockController::isolate()` |
| `controller.disposed` | `BlockController::dispose()` |
| `editor.created` | `BlockRegistry::create_editor()` |
| `interpreter.attached` | `BlockRegistry::attach_interpreter()` |
| `runtime.bound` | `BlockRegistry::bind_runtime()` |
| `orchestrator.submit` | `LocalOrchestrator::submit()` |
| `orchestrator.cancel` | `LocalOrchestrator::cancel()` |
| `orchestrator.replay` | `LocalOrchestrator::replay()` |
| `artifact.produced` | `MeshArtifactStore::produce()` |
| `artifact.consumed` | `MeshArtifactStore::consume()` |
| `artifact.snapshot` | `MeshArtifactStore::snapshot_node()` |
| `artifact.patched` | `MeshArtifactStore::patch()` |
| `node.created` | `NodeGraph::add_node()` |
| `node.updated` | `NodeGraph::update_node()`, `NodeCustomizer::customize()`, `apply_preset()` |
| `node.deleted` | `NodeGraph::remove_node()` |
| `node.undo` | `NodeGraph::undo()` |
| `node.redo` | `NodeGraph::redo()` |

---

## 7. Orchestrator Dispatch Hooks

`LocalOrchestrator` provides four dispatch operations:

| Operation | Method | Effect |
|-----------|--------|--------|
| Submit | `submit(task_id, dimension_id)` | Registers task; emits `Submitted` event |
| Progress | `progress(task_id, percent, message)` | Emits `Progress` event |
| Cancel | `cancel(task_id)` | Marks task terminal; emits `Cancelled` event |
| Replay | `replay(task_id)` | Returns ordered event history |

Subscriptions are live (broadcast channel): `subscribe()` returns a receiver
that sees all future events.  Historical events are accessed via `replay()`.

Dimension mismatch on `submit()` returns `OrchestratorError::DimensionMismatch`.
Operations on a terminal task return `OrchestratorError::AlreadyTerminal`.

---

## 8. Mesh Artifact Write Path

| Operation | Method | Emits |
|-----------|--------|-------|
| Produce | `MeshArtifactStore::produce(artifact)` | `artifact.produced` |
| Consume | `consume(id)` | `artifact.consumed` |
| Snapshot | `snapshot_node(node_id, state, task_id, dim)` | `artifact.snapshot` |
| Patch | `patch(before, after, ops, task_id, dim)` | `artifact.patched` |

All writes broadcast the new `ArtifactId` to subscribers.  See
[`artifact-model.md`](artifact-model.md) for the full provenance schema.

---

## 9. Node Adder and Customizer

### Node adder (`NodeGraph`)

- `add_node(template, name, position, params)` — validates required parameters,
  inserts node, pushes inverse onto undo stack, clears redo stack.
- `remove_node(id)` — removes node; undoable.
- `update_node(id, updates)` — merges parameter updates; undoable.
- `move_node(id, position)` — updates canvas position; undoable.
- `undo()` / `redo()` — navigate the command history.

Every operation emits the appropriate `node.*` ActionLog event.

### Node customizer (`NodeCustomizer`)

- `register_template(template)` — adds a `NodeTemplate` to the registry.
- `apply_preset(node, template_id, preset_name)` — overwrites node parameters
  with a named preset's values.
- `customize(node, task_id, updates)` — applies direct parameter overrides and
  validates required params.
- `validate_params(node)` — checks all required template parameters are present.

---

## 10. References

- [`event-taxonomy.md`](event-taxonomy.md) — ActionLog verb-noun taxonomy.
- [`artifact-model.md`](artifact-model.md) — Artifact provenance schema.
- [`dimension-model.md`](dimension-model.md) — Dimension namespacing rules.
- [`taskid-invariants.md`](taskid-invariants.md) — TaskID monotonicity guarantees.
- [`runtime-rust/crates/ify-controller/`](../../runtime-rust/crates/ify-controller/) — Implementation.
