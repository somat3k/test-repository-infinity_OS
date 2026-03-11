# Agent and Tool Security Policy

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Security team

---

## 1. Purpose

Enforce least privilege for agents and tools using the capability registry.

---

## 2. Capability Tiers

| Tier | Allowed Capabilities | Notes |
|------|----------------------|-------|
| **Tier 0 (System)** | `CAP_ADMIN`, kernel/system caps | Reserved for system agents only. |
| **Tier 1 (Privileged)** | `CAP_READ_SECRETS`, `CAP_DEPLOY`, `CAP_NET_INGRESS` | Requires human approval per task. |
| **Tier 2 (Standard)** | `CAP_SPAWN_TASKS`, `CAP_READ_ARTIFACTS`, `CAP_WRITE_ARTIFACTS` | Default for operator-approved agents. |
| **Tier 3 (Restricted)** | `CAP_READ_ENV`, read-only artifacts | No network or secret access. |

---

## 3. Enforcement

1. Agent templates must declare `required_capabilities`.
2. Runtime verifies requested capabilities are a subset of the dimension grant.
3. All denials are logged via `capability.denied` ActionLog events.

---

## 4. References

- [`docs/architecture/capability-registry.md`](../architecture/capability-registry.md)
- [`docs/architecture/event-taxonomy.md`](../architecture/event-taxonomy.md)
