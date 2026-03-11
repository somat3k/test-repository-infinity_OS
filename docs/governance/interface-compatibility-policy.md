# Interface Compatibility Policy

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Architecture team

---

## 1. Scope

Applies to:
- `kernel-c/include/` ABI headers
- `runtime-rust/crates/ify-ffi/` bindings
- Mesh artifact schemas
- Cross-layer contracts in `docs/architecture/`

---

## 2. Requirements

1. **Backward compatibility** by default; breaking changes require explicit approval and migration plan.
2. **Versioning**: increment schema or ABI versions for any contract change.
3. **Migration**: include upgrade/downgrade notes for consumers.
4. **Verification**: add or update compatibility tests where available.

---

## 3. Compatibility Review Checklist

- [ ] Contract diff reviewed and approved.
- [ ] Version number updated.
- [ ] Migration notes included.
- [ ] Cross-layer owners approved.

---

## 4. References

- [`docs/architecture/block-controller-contract.md`](../architecture/block-controller-contract.md)
- [`docs/architecture/taskid-invariants.md`](../architecture/taskid-invariants.md)
