# Event Taxonomy (ActionLog)

**Status:** `[-]` in progress  
**Epic:** A — Architecture Foundation  
**Owner:** Architecture team

---

## 1. Purpose

The **ActionLog** is the immutable, append-only ledger of every meaningful event in infinityOS.  It provides:

- A **complete audit trail** for governance and security review.
- The raw material for **replay and time-travel debugging**.
- The event stream consumed by telemetry, analytics, and the Kaizen reliability loop.

Every layer of the system — kernel, runtime, canvas, data, agents, deploy — is responsible for emitting ActionLog entries at defined points.

---

## 2. Entry Schema

All ActionLog entries share a common envelope:

```json
{
  "event_id":      "<ArtifactId>",        // unique ID for this log entry
  "event_type":    "<verb>.<noun>",        // see §3
  "occurred_at_ms": 1741600000000,         // Unix epoch milliseconds
  "dimension_id":  "<DimensionId | null>", // null for kernel-level events
  "task_id":       "<TaskId | null>",      // null if not task-scoped
  "actor":         {                       // who triggered the event
    "kind":  "agent | user | system | kernel",
    "id":    "<opaque actor identifier>"
  },
  "causality_id":  "<event_id | null>",    // event that caused this one
  "correlation_id":"<string | null>",      // groups related events (e.g., a user request)
  "payload":       { ... }                 // verb-specific fields (see §3)
}
```

---

## 3. Verb–Noun Taxonomy

### 3.1 Task Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `task.submitted` | A task was accepted into the queue. | `priority`, `queue_depth` |
| `task.started` | Execution began. | `worker_id` |
| `task.paused` | Task suspended at a yield point. | `reason` |
| `task.resumed` | Task resumed from a paused state. | — |
| `task.completed` | Task finished successfully. | `duration_ms`, `artifact_ids[]` |
| `task.failed` | Task terminated with an error. | `error_kind`, `error_message`, `duration_ms` |
| `task.cancelled` | Task was cancelled by a caller or policy. | `cancelled_by`, `reason` |
| `task.retried` | Task was re-submitted after failure. | `attempt_number`, `backoff_ms` |

### 3.2 Dimension Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `dimension.created` | A new dimension was initialized. | `tier`, `capabilities`, `max_concurrent` |
| `dimension.draining` | Shutdown requested; draining tasks. | `reason` |
| `dimension.destroyed` | All resources freed. | `lifetime_ms` |

### 3.3 Artifact Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `artifact.produced` | An artifact was committed to storage. | `artifact_class`, `immutability_tier`, `size_bytes`, `content_hash` |
| `artifact.consumed` | An artifact was read by a task or agent. | `consumer_task_id` |
| `artifact.archived` | An artifact was moved to cold storage. | `storage_tier` |
| `artifact.deleted` | An artifact was soft-deleted. | `reason`, `policy_id` |
| `artifact.schema_upgraded` | Schema migration applied to an artifact. | `from_version`, `to_version` |

### 3.4 Node / Graph Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `node.created` | A canvas node was added. | `node_kind`, `node_id` |
| `node.updated` | A node's parameters changed. | `changed_fields[]` |
| `node.deleted` | A node was removed from the graph. | — |
| `node.linked` | Two nodes were connected. | `source_port`, `target_port` |
| `node.unlinked` | A connection was removed. | `source_port`, `target_port` |
| `graph.serialized` | The full graph was snapshotted. | `schema_version`, `artifact_id` |

### 3.5 Flow Control Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `flow.evaluated` | A flow control decision was evaluated. | `step_id`, `decision`, `matched` |
| `flow.advanced` | Flow advanced to the next step. | `from`, `to` |

### 3.6 Model Runtime Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `model.hyperparameters_adjusted` | Hyperparameters were tuned based on live performance. | `model_id`, `changes`, `metric`, `observed`, `threshold` |
| `model.reload_requested` | A model reload was requested for recovery. | `model_id`, `reload_generation`, `metric`, `observed`, `threshold` |

### 3.7 Replica Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `replica.provisioned` | A kernel replica was provisioned for a model module. | `replica_id`, `model_id`, `module_id` |
| `replica.released` | A kernel replica was released. | `replica_id`, `model_id`, `module_id` |

### 3.8 Chat Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `chat.request_received` | A chat request was received for payload adaptation. | `request_id`, `message` |
| `chat.payload_generated` | A chat payload was generated for execution. | `request_id`, `intent` |

### 3.9 Agent Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `agent.plan_generated` | An agent produced an execution plan. | `plan_artifact_id`, `step_count` |
| `agent.tool_called` | An agent invoked a tool. | `tool_name`, `capability_used` |
| `agent.tool_result` | A tool returned a result. | `tool_name`, `result_artifact_id` |
| `agent.evaluated` | An agent's output was scored. | `evaluator_id`, `score` |

### 3.10 Controller Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `controller.registered` | A blockController was registered. | `controller_kind`, `dimension_id` |
| `controller.linked` | Controller linked to a node or editor. | `target_id`, `target_kind` |
| `controller.isolated` | Controller isolated (sandbox mode). | `reason` |
| `controller.disposed` | Controller lifecycle ended. | `lifetime_ms` |

### 3.11 System / Kernel Events

| Event Type | Description | Required Payload Fields |
|------------|-------------|------------------------|
| `kernel.initialized` | Kernel subsystems started. | `version`, `granted_caps` |
| `kernel.shutdown` | Kernel shutdown sequence began. | `reason` |
| `capability.granted` | Capabilities granted to a dimension. | `capabilities` |
| `capability.denied` | Capability request denied. | `requested_caps`, `reason` |

---

## 4. Causality and Correlation

### Causality Chain

Each entry's `causality_id` points to the `event_id` of the event that directly caused it.  This builds a **causal DAG** suitable for root-cause analysis:

```
user.request ──► agent.plan_generated ──► task.submitted ──► task.started
                                               │
                                               └──► artifact.produced
```

### Correlation ID

The `correlation_id` is a string that groups all events belonging to the same user-facing operation (e.g., one canvas "run" action).  The canvas layer sets the correlation ID and propagates it downward through all sub-tasks.

---

## 5. Retention and Immutability

- ActionLog entries are **immutable** once written.  No update or delete operation is permitted.
- Retention policy is applied at the dimension level.  The default is 90 days for session tiers and indefinite for persistent tiers.
- Entries are stored as tier-2 artifacts (see `artifact-model.md`), guaranteeing hash-chain integrity.

---

## 6. References

- [`artifact-model.md`](artifact-model.md) — ActionLog entries are stored as tier-2 artifacts.
- [`dimension-model.md`](dimension-model.md) — Dimension-scoped retention policy.
- [`taskid-invariants.md`](taskid-invariants.md) — `task_id` field guarantees.
- EPIC T — Telemetry and Observability (distributed tracing integration).
- EPIC K — Kaizen Reliability Loop (ActionLog as the reliability signal source).
