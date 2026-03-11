# Audit Policy for Privileged Tasks

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Security team

---

## 1. Privileged Task Definition

A task is privileged if it uses any of:
- `CAP_READ_SECRETS`
- `CAP_DEPLOY`
- `CAP_PUBLISH_MARKETPLACE`
- `CAP_ADMIN`

---

## 2. Audit Requirements

1. Emit ActionLog entries for every privileged action with `correlation_id` and `causation_id`.
2. Store audit records for a minimum of 12 months.
3. Sign audit bundles and store hashes in the mesh for tamper evidence.
4. Provide an export path for compliance review.

---

## 3. References

- [`docs/architecture/event-taxonomy.md`](../architecture/event-taxonomy.md)
- [`docs/architecture/capability-registry.md`](../architecture/capability-registry.md)
