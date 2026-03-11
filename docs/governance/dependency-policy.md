# Dependency Policy

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Security team

---

## 1. Requirements

1. Pin dependency versions in manifests and lockfiles.
2. Maintain an allowlist of approved licenses.
3. Run vulnerability scanning before release gates.
4. Record dependency updates in the ActionLog.

---

## 2. Vulnerability Scanning

- Use the repository security tooling to scan runtime, kernel, and tooling dependencies.
- High or critical findings block release until mitigated or explicitly waived.

---

## 3. References

- [`docs/governance/release-gates.md`](release-gates.md)
- [`AGENTS.md`](../../AGENTS.md)
