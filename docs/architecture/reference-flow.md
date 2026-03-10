# End-to-End Reference Flow

**Status:** `[-]` in progress  
**Epic:** A — Architecture Foundation  
**Owner:** Architecture team

---

## 1. Purpose

This document defines the canonical **end-to-end execution flow** for the most common infinityOS operation: a user makes a request in the Chat Column, an agent produces a plan, blockControllers coordinate execution through canvas nodes, results are evaluated, and artifacts are surfaced back to the user.

This reference flow is the contract that all layers must implement against.  Every step maps to defined ActionLog events and specific API calls.

---

## 2. Flow Diagram

```
User
 │
 │  1. "Add a data-cleaning node and run it on dataset X"
 ▼
Chat Column (UX)
 │
 │  2. Create correlation_id, emit user.request → ActionLog
 │  3. Forward to bound agent via Performer Runtime
 ▼
Agent (Performer Runtime)
 │
 │  4. Read dimension capabilities, available tools, memory context
 │  5. Generate plan (steps → tasks → node operations)
 │  6. Emit agent.plan_generated → ActionLog
 ▼
blockControllerGenerator
 │
 │  7. Receive plan steps; instantiate blockControllers per step
 │  8. Emit controller.registered for each controller
 │  9. Link controllers to existing/new canvas nodes
 │  10. Emit controller.linked
 ▼
Canvas Nodes
 │
 │  11. Node parameters set; node marked QUEUED in canvas UI
 │  12. Emit node.created / node.updated → ActionLog
 ▼
Performer Runtime (Task Execution)
 │
 │  13. Submit tasks to Executor (one per node / step)
 │  14. Emit task.submitted → ActionLog
 │  15. Executor dispatches tasks (priority-aware, rate-limited)
 │  16. Emit task.started → ActionLog
 ▼
Kernel (C)
 │
 │  17. Scheduler runs task function
 │  18. Memory arena allocated for task scratch space
 │  19. TaskID generated via ify_task_id_generate()
 ▼
Task Execution
 │
 │  20. Load artifact (dataset X) from mesh
 │  21. Emit artifact.consumed → ActionLog
 │  22. Execute data-cleaning transform
 │  23. Write output artifact to mesh
 │  24. Emit artifact.produced → ActionLog
 │  25. Emit task.completed → ActionLog
 ▼
blockController (Orchestration)
 │
 │  26. Receive task.completed notification
 │  27. Verify artifact provenance and content hash
 │  28. Update node state → COMPLETED in canvas
 │  29. Emit node.updated → ActionLog
 ▼
Agent (Evaluation)
 │
 │  30. Read output artifact
 │  31. Score result against evaluation criteria
 │  32. Emit agent.evaluated → ActionLog
 │  33. If score < threshold → retry (emit task.retried)
 ▼
Chat Column (UX)
 │
 │  34. Display result summary, artifact preview, and score to user
 │  35. User can accept, reject, or request revision
 └──► End
```

---

## 3. Step-by-Step Reference

### Step 1–3 — User Request Intake

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 1 | User | Enters text in Chat Column. | — |
| 2 | Chat Column | Allocates `correlation_id` (UUID v4); creates UX-side request object. | *(internal)* |
| 3 | Chat Column | Serializes request with `correlation_id` and forwards to agent API. | — |

### Step 4–6 — Agent Planning

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 4 | Agent | Reads dimension capabilities, available tools, and memory context via runtime API. | — |
| 5 | Agent | Generates plan: ordered list of steps with tool calls, node operations, and expected artifacts. | — |
| 6 | Agent | Persists plan as tier-1 artifact; emits event. | `agent.plan_generated` |

**Plan schema (abbreviated):**
```json
{
  "plan_id": "<ArtifactId>",
  "correlation_id": "<string>",
  "steps": [
    {
      "step_id": 1,
      "description": "Create data-cleaning node",
      "kind": "node_create",
      "node_kind": "DataTransform",
      "parameters": { "transform": "clean_nulls", "input_artifact": "<ArtifactId>" }
    },
    {
      "step_id": 2,
      "description": "Execute node",
      "kind": "task_submit",
      "depends_on": [1]
    }
  ]
}
```

### Step 7–10 — blockController Instantiation

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 7 | blockControllerGenerator | Receives plan; instantiates one controller per step. | — |
| 8 | Controller | Registers with the runtime. | `controller.registered` |
| 9 | Controller | Links to the target canvas node (creates node if it doesn't exist). | — |
| 10 | Controller | Emits link event. | `controller.linked` |

### Step 11–16 — Node Setup and Task Submission

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 11 | Canvas | Sets node parameters from controller config; renders node as QUEUED. | — |
| 12 | Canvas | Persists graph mutation. | `node.created` / `node.updated` |
| 13 | Performer Runtime | Calls `Executor::submit()` for each node task. | — |
| 14 | Executor | Records task in queue. | `task.submitted` |
| 15 | Executor | Dispatches task to a worker (respects concurrency limits). | — |
| 16 | Executor | Worker begins execution. | `task.started` |

### Step 17–19 — Kernel Execution

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 17 | Kernel Scheduler | Runs the task function on the worker thread. | — |
| 18 | Kernel Memory | Allocates task-scoped arena for scratch space. | — |
| 19 | Kernel | Generates `TaskId` via `ify_task_id_generate()`. | — |

### Step 20–25 — Task Execution

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 20 | Task | Reads input artifact from mesh. | — |
| 21 | Task | Records read. | `artifact.consumed` |
| 22 | Task | Executes the data-cleaning transform. | — |
| 23 | Task | Writes output artifact to mesh. | — |
| 24 | Task | Records write. | `artifact.produced` |
| 25 | Task | Signals completion to Executor. | `task.completed` |

### Step 26–29 — Controller Reconciliation

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 26 | blockController | Receives `task.completed` notification via subscription. | — |
| 27 | Controller | Verifies artifact `content_hash` against provenance record. | — |
| 28 | Controller | Updates node state to COMPLETED; pushes update to canvas. | — |
| 29 | Canvas | Re-renders node. | `node.updated` |

### Step 30–33 — Agent Evaluation

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 30 | Agent | Reads output artifact from mesh. | `artifact.consumed` |
| 31 | Agent | Scores result (LLM judge or deterministic metric). | — |
| 32 | Agent | Records evaluation result. | `agent.evaluated` |
| 33 | Agent | If score below threshold → re-plan or retry with adjusted params. | `task.retried` |

### Step 34–35 — Result Surface

| Step | Actor | Action | ActionLog Event |
|------|-------|--------|----------------|
| 34 | Chat Column | Receives evaluation summary; renders result preview. | — |
| 35 | User | Accepts, rejects, or requests revision; loop restarts from Step 3. | — |

---

## 4. Invariants

1. Every step that mutates state emits a corresponding ActionLog entry before returning.
2. The `correlation_id` from Step 2 is propagated to **every** ActionLog entry in the chain.
3. No step may proceed if a prerequisite step's required artifact is missing or has a hash mismatch.
4. Retry loops (Step 33) are bounded by the agent's `max_retries` policy (default: 3).
5. Task cancellation propagates immediately: a `task.cancelled` event terminates the controller reconciliation chain.

---

## 5. References

- [`dimension-model.md`](dimension-model.md) — Dimension scoping for tasks and artifacts.
- [`taskid-invariants.md`](taskid-invariants.md) — TaskID generation in Step 19.
- [`artifact-model.md`](artifact-model.md) — Artifact production and consumption in Steps 20–24.
- [`event-taxonomy.md`](event-taxonomy.md) — Full ActionLog event schema for all steps above.
- [`capability-registry.md`](capability-registry.md) — Capability checks in Steps 4 and 7.
- [`ux-surface-map.md`](ux-surface-map.md) — Chat Column and Canvas integration points.
- EPIC B — blockControllerGenerator Regime (Steps 7–10 and 26–29).
- EPIC R — Rust Performer Runtime (Steps 13–16 executor implementation).
- EPIC C — C Kernel and Boost Layer (Steps 17–19).
