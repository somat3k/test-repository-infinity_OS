# Dimension Model

**Status:** `[-]` in progress  
**Epic:** A — Architecture Foundation  
**Owner:** Architecture team

---

## 1. What Is a Dimension?

A **dimension** is the fundamental isolation boundary in infinityOS.  Every piece of execution — tasks, agents, artifacts, node graphs, and data pipelines — lives inside a dimension.  Dimensions enforce:

- **Namespace isolation**: identifiers (TaskIDs, ArtifactIDs, node IDs) are unique within a dimension and never collide across dimensions without an explicit relay.
- **Tenancy boundaries**: multiple users, projects, or environments can coexist on the same runtime without interference.
- **Scope boundaries**: capabilities, rate limits, and quotas are applied per-dimension.

A dimension is **not** a process or a container.  It is a logical grouping enforced at the kernel and runtime level.  A single OS process may host many dimensions simultaneously.

---

## 2. Dimension Lifecycle

```
create()
   │
   ▼
ACTIVE  ──►  DRAINING  ──►  DESTROYED
               (all tasks drained or timed-out)
```

| State | Description |
|-------|-------------|
| `ACTIVE` | Normal operating state.  Tasks may be submitted and artifacts written. |
| `DRAINING` | Shutdown requested.  No new tasks accepted; existing tasks run to completion. |
| `DESTROYED` | All resources released.  The `DimensionId` is permanently retired. |

---

## 3. Namespace Scoping

Every resource in infinityOS is qualified by its owning dimension:

```
{DimensionId} / {TaskId}
{DimensionId} / {ArtifactId}
{DimensionId} / nodes / {NodeId}
```

Cross-dimension access is forbidden unless an explicit **cross-dimension relay** is configured by an operator.  Relays are modelled as special task pairs that bridge artifact reads/writes.

---

## 4. Tenancy Model

Dimensions support three tenancy tiers:

| Tier | Description |
|------|-------------|
| **Project** | One dimension per user project.  Default isolation level. |
| **Environment** | One dimension per deployment environment (dev/staging/prod). |
| **Micro-dimension** | Ephemeral dimension for a single task or snippet execution.  Destroyed on task completion. |

Tenancy tier is set at dimension creation and cannot be changed.

---

## 5. Scope Boundaries and Capability Enforcement

At creation, a dimension is assigned a **capability profile** drawn from the granted kernel capabilities.  A dimension cannot exercise a capability that was not included in its profile.

```
Kernel-granted caps: {MEMORY, SCHEDULER, FS, NET}
                         │
               ┌─────────┴─────────┐
               │  Dimension A      │  caps: {MEMORY, SCHEDULER}
               │  Dimension B      │  caps: {MEMORY, SCHEDULER, FS}
               └───────────────────┘
```

Capability profiles are immutable after dimension creation.  If additional capabilities are required, a new dimension must be created.

---

## 6. DimensionId Format

A `DimensionId` is a UUID v4 (randomly generated), encoded as a standard 36-character hyphenated string:

```
xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
```

`DimensionId`s are:
- **globally unique** (UUID v4 collision probability is negligible).
- **opaque**: consumers must not interpret the internal layout.
- **permanent**: a retired `DimensionId` is never reused.

---

## 7. Open Questions

- Should micro-dimensions reuse backing arenas to reduce allocation pressure?
- Should cross-dimension relays be implemented as kernel primitives or runtime constructs?
- Rate-limit policy for dimension creation (prevent DoS via dimension flooding).

---

## 8. References

- [`taskid-invariants.md`](taskid-invariants.md) — TaskID uniqueness within and across dimensions.
- [`capability-registry.md`](capability-registry.md) — Capability taxonomy and grant rules.
- [`kernel-c/include/infinity/kernel.h`](../../kernel-c/include/infinity/kernel.h) — `ify_dimension_id_t`, `ify_dimension_create()`, `ify_dimension_destroy()`.
- [`runtime-rust/crates/ify-core/src/lib.rs`](../../runtime-rust/crates/ify-core/src/lib.rs) — `DimensionId` Rust type.
