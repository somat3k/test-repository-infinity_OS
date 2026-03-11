# Incident Response Process

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Reliability team

---

## 1. Incident Triggers

- Suspected agent/tool compromise
- Unauthorized capability escalation
- Data leakage or integrity breach

---

## 2. Response Phases

1. **Detect**: capture logs, identify scope, lock down access.
2. **Contain**: revoke credentials, disable affected agents, freeze dimensions.
3. **Eradicate**: remove malicious artifacts, patch vulnerabilities.
4. **Recover**: restore services, monitor for recurrence.
5. **Review**: post-incident report with root cause and remediation plan.

---

## 3. References

- [`docs/governance/audit-policy.md`](audit-policy.md)
- [`docs/architecture/event-taxonomy.md`](../architecture/event-taxonomy.md)
