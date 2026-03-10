# Artifact Model

**Status:** `[-]` in progress  
**Epic:** A — Architecture Foundation  
**Owner:** Architecture team

---

## 1. What Is an Artifact?

An **artifact** is an immutable, versioned output produced by a task.  Every write to the mesh or a node that persists beyond the task's lifetime becomes an artifact.  Artifacts are the primary mechanism for:

- Passing data between tasks without shared mutable state.
- Providing a complete, auditable history of every computation.
- Enabling replay, debugging, and root-cause analysis.

---

## 2. Artifact Classes

| Class | Description |
|-------|-------------|
| **Mesh Artifact** | Output written to the shared mesh data canvas (node results, graph state snapshots). |
| **Node Artifact** | Output scoped to a single canvas node (intermediate computation, node-local state). |
| **Task Artifact** | Structured output emitted by a task (JSON, binary blob, log stream). |
| **Agent Artifact** | Policy-controlled output produced by an agent (plans, evaluations, model responses). |

---

## 3. Immutability Tiers

Artifacts are classified into three immutability tiers that control retention, mutation, and garbage collection:

| Tier | Name | Description |
|------|------|-------------|
| 0 | **Ephemeral** | Exists only for the duration of a task.  Automatically destroyed on task completion.  Never persisted to disk. |
| 1 | **Session** | Retained for the duration of a dimension session.  Destroyed when the dimension is torn down.  May be spilled to disk under memory pressure. |
| 2 | **Persistent** | Retained indefinitely until explicitly archived or deleted.  Always persisted to disk with integrity verification. |

Tier is set at artifact creation and is **immutable**.

---

## 4. ArtifactId Format

An `ArtifactId` is a **UUID v7**, using the same time-ordered format as `TaskId`:

```
xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx
```

`ArtifactId`s are:
- **globally unique** across all dimensions and artifact classes.
- **time-ordered**, enabling efficient range queries (e.g., "all artifacts produced in the last hour").
- **permanent**: retired IDs are never reused.

---

## 5. Provenance Record

Every artifact carries a **provenance record** linking it to the execution chain that produced it:

```json
{
  "artifact_id": "<ArtifactId>",
  "dimension_id": "<DimensionId>",
  "producing_task_id": "<TaskId>",
  "producing_agent_id": "<AgentId | null>",
  "producing_node_id": "<NodeId | null>",
  "controller_id": "<ControllerId | null>",
  "created_at_ms": 1741600000000,
  "artifact_class": "mesh",
  "immutability_tier": 2,
  "content_hash": "sha256:<hex>",
  "size_bytes": 4096,
  "schema_version": "1.0.0"
}
```

Provenance records are themselves immutable persistent artifacts (tier 2) and are indexed by every field for efficient forensic queries.

---

## 6. Content Addressing

Artifact content is referenced by a **SHA-256 content hash**.  Two artifacts with the same content hash are byte-identical.  The storage layer may deduplicate them at the block level while retaining distinct `ArtifactId`s in the index for provenance accuracy.

### 6.1 IPFS Storage Policy for Legal/Regulatory Artifacts

Tier 2 persistent artifacts representing **TeraForms, contracts, licenses, certifications, legal documents, and regulatory filings** are stored through the IPFS storage adapter. The adapter derives a CIDv1 from the `content_hash` and records the CID in storage metadata for retrieval and pinning policies. IPFS storage never mutates artifact payloads; it only provides a content-addressed backing store aligned with the existing immutability tiers.

---

## 7. Artifact Lifecycle

```
produce()
    │
    ▼
PENDING  ──►  COMMITTED  ──►  ARCHIVED  ──►  DELETED
                                (tier 2 only)     (explicit or policy-driven)
```

| State | Description |
|-------|-------------|
| `PENDING` | Written to the staging buffer; not yet durable. |
| `COMMITTED` | Flushed to durable storage; content hash verified. |
| `ARCHIVED` | Moved to cold storage tier; still addressable by `ArtifactId`. |
| `DELETED` | Soft-deleted; `ArtifactId` is retained in the tombstone index for 90 days. |

---

## 8. Artifact Schema Versioning

All structured artifacts carry a `schema_version` field following semantic versioning (`MAJOR.MINOR.PATCH`).  Readers must:

1. Reject artifacts with a higher MAJOR version than supported.
2. Tolerate unknown fields in MINOR/PATCH versions (forward compatibility).
3. Never mutate an artifact's content; create a new artifact with a new schema version if transformation is required.

---

## 9. References

- [`dimension-model.md`](dimension-model.md) — Artifact namespace scoping.
- [`taskid-invariants.md`](taskid-invariants.md) — Producing task invariants.
- [`event-taxonomy.md`](event-taxonomy.md) — ActionLog entries for artifact lifecycle events.
- [`runtime-rust/crates/ify-core/src/lib.rs`](../../runtime-rust/crates/ify-core/src/lib.rs) — `ArtifactId` Rust type.
