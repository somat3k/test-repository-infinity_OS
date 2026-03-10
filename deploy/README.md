# deploy — Deployment Adapters and Manifests

The deploy layer owns deployment targets, workload distribution, canary rollouts, and the adapters that connect infinityOS execution to external infrastructure.

## Responsibilities

- Deployment targets: local, cluster, hybrid
- Worker registration and capability matching
- Artifact transport between workers
- Autoscaling hooks based on queue depth and SLO state
- Multi-tenant isolation boundaries
- Canary deployments for runtime updates
- Disaster recovery plan and documentation
- Deployment UI workflow integration

## Constraints

- **Depends on Performer Runtime and Data**: no direct calls to canvas, agents, or kernel layers.
- **Isolation guarantees**: every tenant workload must be fully isolated from other tenants in shared deployments.
- **Auditable rollouts**: every deployment action is recorded in the ActionLog with TaskID, dimension, and actor.
- **Rollback-safe**: every deployment must support a defined rollback path; destructive operations require explicit confirmation.

## Epic Tracking

See [EPIC W — Workload Distribution and Deployment](../TODO.md) in `TODO.md`.
