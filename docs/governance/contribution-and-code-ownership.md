# Contribution and Code Ownership Policy

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Governance team

---

## 1. Purpose

Define review and ownership requirements for every layer so cross-layer contracts remain stable.

---

## 2. CODEOWNERS

1. `/CODEOWNERS` is the authoritative owner map.
2. Every top-level layer (kernel, runtime, canvas, data, agents, deploy, docs) must have an owner.
3. Ownership changes require governance approval.

---

## 3. Review Rules

1. All PRs require at least one approval from each touched code owner.
2. Cross-layer interface changes require an architecture owner approval (kernel ABI, runtime FFI, mesh schemas).
3. Capability, audit, or incident policies require governance owner approval.
4. Emergency changes must include a follow-up remediation PR within 24 hours.

---

## 4. References

- [`/CODEOWNERS`](../../CODEOWNERS)
- [`AGENTS.md`](../../AGENTS.md)
