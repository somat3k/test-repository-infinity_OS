# Runtime Capability Registry

**Status:** `[-]` in progress  
**Epic:** A — Architecture Foundation  
**Owner:** Architecture team

---

## 1. Purpose

The **capability registry** is the single source of truth for what every component in infinityOS is allowed to do.  It enforces the principle of least privilege: every agent, task, and dimension must explicitly declare and be granted the capabilities it needs.  Undeclared capabilities are denied by default.

---

## 2. Capability Taxonomy

Capabilities are organised into four tiers: **Hardware**, **System**, **Runtime**, and **Application**.

### 2.1 Hardware Capabilities

Discovered at kernel boot via hardware enumeration.  These are the root capabilities from which all others are derived.

| Capability | Bitmask | Description |
|------------|---------|-------------|
| `CAP_MEMORY` | `1 << 0` | Access to the kernel memory allocator. |
| `CAP_SCHEDULER` | `1 << 1` | Access to the task scheduler. |
| `CAP_PERF` | `1 << 4` | Hardware performance counter access. |
| `CAP_GPU` | `1 << 5` | GPU / accelerator access. |

### 2.2 System Capabilities

Granted based on OS-level permissions and sandbox configuration.

| Capability | Bitmask | Description |
|------------|---------|-------------|
| `CAP_FS` | `1 << 2` | Sandboxed filesystem access (read-only by default). |
| `CAP_FS_WRITE` | `1 << 8` | Filesystem write access (requires explicit grant). |
| `CAP_NET` | `1 << 3` | Sandboxed network access (egress-only by default). |
| `CAP_NET_INGRESS` | `1 << 9` | Network ingress (listen) access. |

### 2.3 Runtime Capabilities

Granted at dimension creation based on the operator's security policy.

| Capability | Bitmask | Description |
|------------|---------|-------------|
| `CAP_SPAWN_TASKS` | `1 << 16` | Permission to submit tasks to the scheduler. |
| `CAP_SPAWN_DIMENSIONS` | `1 << 17` | Permission to create child dimensions. |
| `CAP_READ_ARTIFACTS` | `1 << 18` | Read artifacts from the mesh. |
| `CAP_WRITE_ARTIFACTS` | `1 << 19` | Write artifacts to the mesh. |
| `CAP_INVOKE_TOOLS` | `1 << 20` | Invoke registered tools (DB, HTTP, model, blockchain). |
| `CAP_INVOKE_MODEL` | `1 << 21` | Invoke ML model inference. |
| `CAP_DEPLOY` | `1 << 22` | Trigger deployment workflows. |

### 2.4 Application Capabilities

Declared by agent templates; enforced by the agent sandbox layer.

| Capability | Bitmask | Description |
|------------|---------|-------------|
| `CAP_READ_ENV` | `1 << 32` | Read environment configuration (non-secret). |
| `CAP_READ_SECRETS` | `1 << 33` | Read secrets (requires operator approval). |
| `CAP_PUBLISH_MARKETPLACE` | `1 << 34` | Publish to the agent/snippet marketplace. |
| `CAP_ADMIN` | `1 << 63` | Full administrative access (reserved for system agents). |

---

## 3. Capability Grant Rules

1. **Kernel root grant**: at boot, the kernel enumerates hardware and grants all available hardware capabilities to the host process.
2. **Dimension grant**: when a dimension is created, the operator specifies a capability profile.  A dimension cannot be granted capabilities the host process does not have.
3. **Task grant**: when a task is submitted, it inherits the owning dimension's capabilities minus any explicitly revoked subset.
4. **Agent grant**: agent templates declare a `required_capabilities` list.  The runtime verifies that the owning dimension grants all required capabilities before instantiating the agent.
5. **Propagation**: child dimensions inherit at most the parent's capability set.  Capabilities cannot be escalated.

---

## 4. Capability Verification Flow

```
agent.instantiate(template)
        │
        ▼
    Check: template.required_capabilities ⊆ dimension.granted_capabilities
        │
        ├── YES ──► proceed
        └── NO  ──► emit capability.denied ActionLog entry
                    return CapabilityDenied error to caller
```

---

## 5. Capability Change Policy

- Hardware capabilities are **fixed** after kernel boot.
- Dimension capabilities are **fixed** after dimension creation.
- Agent/task capability profiles can be updated for future instantiations, but **not** for already-running tasks.
- Any capability change is recorded in the ActionLog with the actor, timestamp, and justification.

---

## 6. Audit Requirements

All capability grant and deny events must be recorded in the ActionLog:

- `capability.granted` — emitted when a dimension or task is initialized with a non-empty capability set.
- `capability.denied` — emitted for every capability check failure.

See [`event-taxonomy.md`](event-taxonomy.md) §3.7 for the required payload fields.

---

## 7. References

- [`kernel-c/include/infinity/kernel.h`](../../kernel-c/include/infinity/kernel.h) — `ify_capabilities_t`, `IFY_CAP_*` constants.
- [`runtime-rust/crates/ify-core/src/lib.rs`](../../runtime-rust/crates/ify-core/src/lib.rs) — `Capabilities` Rust bitflags type.
- [`event-taxonomy.md`](event-taxonomy.md) — ActionLog entries for capability events.
- EPIC O — Operational Security (capability enforcement and audit requirements).
