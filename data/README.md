# data — Archival and Storage Pipelines

The data layer owns archival policies, storage adapters, transformation pipelines, schema versioning, lineage tracking, and dataset replay.

## Responsibilities

- Hot/warm/cold archival tiers per dimension and project
- Retention settings and legal holds per artifact class
- Storage adapters (local fs, object store, database-backed, IPFS) with uniform API
- IPFS storage for TeraForms, contracts, licenses, certifications, and legal/regulatory documents (tier 2 persistent)
- Multimedia database interpretation for extended schemas and cross-media indexing
- Schema-driven automations, retention rules, and circular analysis executed by predefined agents
- Dataset versioning (semantic tags + immutable snapshots)
- Hash-chain integrity verification and restoration checks
- Provenance linking: artifact → producing task → agent → controller → node graph
- Backup/restore tooling (CLI + in-app) with dry-run support
- Deduplication strategy for large artifacts (content-addressed chunks)
- Processing and transformation primitives (map/filter/reduce/aggregate/window)
- DeFi resource pooling, encapsulation planning, circuit computation metadata, and distributed execution across connected node networks
- Dead-letter handling, streaming mode, watermarking, checkpointing
- Encryption-at-rest hooks and key management integration
- Performance benchmarks for ingest/query/restore operations

## Constraints

- **Depends only on Performer Runtime**: no direct calls to canvas, agents, or deploy layers.
- **Backward-compatible migrations**: data schema changes must include a migration path; destructive changes are forbidden without a major version bump.
- **Immutable event history**: ActionLog entries must never be mutated after commit.
- **Measurable budgets**: every adapter must expose latency and throughput metrics.

## Epic Tracking

See [EPIC D — Data Archiving and Storage](../TODO.md) and [EPIC P — Processing and Transformation Pipelines](../TODO.md) in `TODO.md`.
