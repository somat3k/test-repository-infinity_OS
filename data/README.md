# data — Archival and Storage Pipelines

The data layer owns archival policies, storage adapters, transformation pipelines, schema versioning, lineage tracking, and dataset replay.

## Responsibilities

- Hot/warm/cold archival tiers per dimension and project
- Storage adapters (local fs, object store, database-backed) with uniform API
- Dataset versioning (semantic tags + immutable snapshots)
- Hash-chain integrity verification and restoration checks
- Provenance linking: artifact → producing task → agent → controller → node graph
- Processing and transformation primitives (map/filter/reduce/aggregate/window)
- Dead-letter handling, streaming mode, watermarking, checkpointing
- Encryption-at-rest hooks and key management integration

## Constraints

- **Depends only on Performer Runtime**: no direct calls to canvas, agents, or deploy layers.
- **Backward-compatible migrations**: data schema changes must include a migration path; destructive changes are forbidden without a major version bump.
- **Immutable event history**: ActionLog entries must never be mutated after commit.
- **Measurable budgets**: every adapter must expose latency and throughput metrics.

## Epic Tracking

See [EPIC D — Data Archiving and Storage](../TODO.md) and [EPIC P — Processing and Transformation Pipelines](../TODO.md) in `TODO.md`.
