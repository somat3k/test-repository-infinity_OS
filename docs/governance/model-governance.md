# Model Governance Policy

**Status:** `[x]`  
**Epic:** G — Governance and Policies  
**Owner:** Model governance

---

## 1. Scope

Applies to model training, fine-tuning, and inference used by agents and workflows.

---

## 2. Requirements

1. Only allow models from approved registries.
2. Maintain lineage for datasets, prompts, and evaluation artifacts.
3. Run pre-deployment evaluation with defined success thresholds.
4. Record model version, owner, and evaluation summary in the mesh.

---

## 3. References

- [`docs/architecture/artifact-model.md`](../architecture/artifact-model.md)
- [`docs/architecture/reference-flow.md`](../architecture/reference-flow.md)
