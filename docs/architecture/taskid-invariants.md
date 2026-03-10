# TaskID Invariants

**Status:** `[-]` in progress  
**Epic:** A — Architecture Foundation  
**Owner:** Architecture team

---

## 1. Purpose

`TaskId` is the single most important identifier in infinityOS.  Every task, every artifact, every ActionLog entry, and every agent action is traceable back to a `TaskId`.  Getting the invariants right from day one prevents a class of bugs (ID collision, ordering inversion, tenant leakage) that are extremely costly to fix retroactively.

---

## 2. Format

A `TaskId` is a **UUID version 7** encoded as a standard 36-character hyphenated string:

```
xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx
└──────────────────────────────────────┘
         timestamp_ms (48 bits)
                     │  ver (4 bits, always 0x7)
                     │       │  rand_a (12 bits)
                     │       │           var (2 bits)
                     │       │               rand_b (62 bits)
```

UUID v7 was chosen because:
- The 48-bit ms timestamp prefix provides **natural sort order** (database index efficiency).
- 74 bits of randomness provide negligible collision probability even at 10 M IDs/second.
- It is an IETF standard (RFC 9562), supported by major UUID libraries in all target languages.

---

## 3. Invariants

### 3.1 Global Uniqueness

> **Every `TaskId` is unique across all dimensions, runtimes, and time.**

The UUID v7 format guarantees this through the combination of timestamp and random bits.  The kernel's `ify_task_id_generate()` function additionally records the last generated ID per-dimension and increments the random portion if two IDs would fall within the same millisecond tick, ensuring strict monotonicity even in high-frequency scenarios.

### 3.2 Per-Dimension Monotonicity

> **Within a single dimension, `TaskId`s generated sequentially are non-decreasing.**

This is guaranteed by the UUID v7 timestamp prefix.  If two IDs are generated within the same millisecond, the kernel increments the `rand_a` field to maintain ordering.  Overflow of `rand_a` within a single millisecond is treated as a fatal error (should never occur in practice; requires > 4,095 IDs in 1 ms from the same dimension).

### 3.3 No Reuse

> **A `TaskId` is never reused, even after the owning task has completed or been destroyed.**

The kernel maintains a per-dimension high-water mark.  A new `TaskId` must always be greater than the high-water mark.  Attempting to generate an ID below the high-water mark returns `IFY_ERR_OVERFLOW`.

### 3.4 Tenant Opacity

> **A `TaskId` does not embed the owning `DimensionId`.**

Although tasks are always owned by a dimension, the `TaskId` itself carries no tenant information.  Tenant context is provided separately in the ActionLog and artifact provenance records.  This prevents information leakage when IDs are exposed through APIs or logs.

### 3.5 Immutability

> **Once assigned, a `TaskId` never changes.**

Tasks may transition through lifecycle states (QUEUED → RUNNING → COMPLETED), but their ID remains constant throughout.

---

## 4. Collision Strategy

UUID v7 collision probability at sustained 10 M IDs/second across all dimensions is approximately 10⁻²⁵ per second — effectively zero for any realistic workload.  No additional deduplication layer is required in the happy path.

In the rare event of a detected collision (e.g., during state recovery from a crash), the kernel returns `IFY_ERR_ALREADY_EXISTS` and the caller must retry with a new generation.

---

## 5. Derivation Option (Deterministic IDs)

For replay and idempotent task submission, callers may provide a deterministic seed to derive a `TaskId`:

```
TaskId = UUIDv5(namespace=INFINITY_TASK_NS, name="{DimensionId}/{seed_string}")
```

Where `INFINITY_TASK_NS` is the fixed namespace UUID:

```
6ba7b810-9dad-11d1-80b4-00c04fd430c8  (DNS namespace, repurposed as a stable root)
```

Deterministic IDs are marked with a well-known variant bit so the kernel can identify them and skip high-water mark enforcement.

---

## 6. References

- [`dimension-model.md`](dimension-model.md) — Dimension namespace scoping.
- [`kernel-c/include/infinity/kernel.h`](../../kernel-c/include/infinity/kernel.h) — `ify_task_id_t`, `ify_task_id_generate()`, `ify_task_id_to_str()`.
- [`runtime-rust/crates/ify-core/src/lib.rs`](../../runtime-rust/crates/ify-core/src/lib.rs) — `TaskId` Rust type and tests.
- RFC 9562 — Universally Unique IDentifiers (UUIDs), Section 5.7 (UUID Version 7).
