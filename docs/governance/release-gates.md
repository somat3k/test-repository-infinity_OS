# Release Gates

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Release engineering

---

## 1. Purpose

Define non-negotiable gates before releasing kernel, runtime, or data changes.

---

## 2. Gate Checklist

1. **Scope classification**: kernel/runtime/data/cross-layer.
2. **Layer tests**: run the full test suite for affected layers.
3. **Interface compatibility**: required for ABI, FFI, schema, or mesh changes.
4. **Security review**: dependency scan + capability review for new privileges.
5. **Reliability sign-off**: regressions, perf budgets, and incident impact assessed.

---

## 3. Evidence

- Gate status is recorded in the ActionLog as `release.gate.passed` or `release.gate.failed` with reviewer and timestamp.

---

## 4. References

- [`interface-compatibility-policy.md`](interface-compatibility-policy.md)
- [`AGENTS.md`](../../AGENTS.md)
