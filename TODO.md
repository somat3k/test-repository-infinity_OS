# infinityOS — TODO (A–Z Epics)

Status legend: `[ ]` pending, `[-]` in progress, `[x]` complete

## A. Architecture Foundation
- [x] Define layered architecture map (kernel, runtime, canvas, data, deploy). (Owner: copilot, 2026-03-10)
- [x] Freeze module boundaries and dependency rules. (Owner: copilot, 2026-03-10)
- [x] Publish system context and component diagrams. (Owner: copilot, 2026-03-10)
- [x] Specify dimension model (what is a dimension, namespaces, tenancy, scope boundaries). (Owner: copilot, 2026-03-10) → docs/architecture/dimension-model.md
- [x] Define TaskID invariants (unique per dimension, stable derivation, collision strategy). (Owner: copilot, 2026-03-10) → docs/architecture/taskid-invariants.md
- [x] Define artifact model (mesh artifacts, node artifacts, provenance, immutability tiers). (Owner: copilot, 2026-03-10) → docs/architecture/artifact-model.md
- [x] Write event taxonomy (ActionLog verbs, payload schema, causality, correlation IDs). (Owner: copilot, 2026-03-10) → docs/architecture/event-taxonomy.md
- [x] Define runtime capability registry (features, permissions, hardware, sandbox capabilities). (Owner: copilot, 2026-03-10) → docs/architecture/capability-registry.md
- [x] Establish product UX surface map (multiplex agents window, project window, chat column, widgets, editors, node canvas). (Owner: copilot, 2026-03-10) → docs/architecture/ux-surface-map.md
- [x] Define end-to-end "user request → agent plan → blockControllers → nodes → execution → evaluation" reference flow. (Owner: copilot, 2026-03-10) → docs/architecture/reference-flow.md

## B. blockControllerGenerator Regime
- [x] Specify dimensional block controller contracts (inputs/outputs, dimension scoping, invariants). (Owner: copilot, 2026-03-10) → docs/architecture/block-controller-contract.md
- [x] Implement controller lifecycle: create → link → isolate → dispose (with deterministic cleanup). (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/controller.rs
- [x] Validate invalid dimensional mappings (type checks + runtime guards + error surfaces). (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/controller.rs
- [x] Implement global-per-dimension TaskID allocator (monotonic + ULID/UUIDv7) + deterministic derivation option. (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/task_allocator.rs
- [x] Implement ActionLog capture for every controller action (register block → editor create → interpreter attach → orchestration → mesh updates). (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/action_log.rs
- [x] Implement block registration pipeline: create new editor instance, attach interpreter, bind to runtime. (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/registry.rs
- [x] Implement orchestrator dispatch hooks (submit tasks, subscribe to progress, cancel, replay). (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/orchestrator.rs
- [x] Implement mesh-artifact write path (produce/consume artifacts, node snapshots, diff patches). (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/mesh.rs
- [x] Implement seamless node adder (from code editor + user intent) with undo/redo and validation. (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/node.rs
- [x] Implement node customizer (parameters/tools/memory/task-flow wiring) with templates and presets. (Owner: copilot, 2026-03-10) → runtime-rust/crates/ify-controller/src/node.rs

## C. C Kernel and Boost Layer
- [x] Establish kernel library layout in C (modules, headers, build system, tests). (Owner: copilot, 2026-03-10) → kernel-c/CMakeLists.txt, kernel-c/src/, kernel-c/tests/
- [x] Implement scheduler baseline primitives (queues, priorities, timers, cooperative yield). (Owner: copilot, 2026-03-10) → kernel-c/src/scheduler.c, kernel-c/include/infinity/scheduler.h
- [x] Implement memory baseline primitives (allocators, arenas, refcount/RCU option, bounds checking). (Owner: copilot, 2026-03-10) → kernel-c/src/memory.c, kernel-c/include/infinity/memory.h
- [x] Add ABI-safe export surface for Rust performer layer (FFI types, versioning, compatibility tests). (Owner: copilot, 2026-03-10) → kernel-c/src/ffi.c, kernel-c/include/infinity/ffi.h, kernel-c/tests/test_ffi.c
- [x] Implement kernel boot sequence (init → capability discovery → subsystem start → service loop). (Owner: copilot, 2026-03-10) → kernel-c/src/kernel.c
- [x] Implement kernel service registry (named services, lifecycle, health checks). (Owner: copilot, 2026-03-10) → kernel-c/src/service_registry.c, kernel-c/include/infinity/service_registry.h
- [x] Implement replication kernel (task-scoped micro-kernel instances for specified workloads). (Owner: copilot, 2026-03-10) → kernel-c/src/replication.c, kernel-c/include/infinity/replication.h
- [x] Define replication policies (when to replicate, resource caps, teardown, pinning). (Owner: copilot, 2026-03-10) → kernel-c/include/infinity/replication.h (ify_replica_policy_t), kernel-c/src/replication.c
- [x] Add kernel tracing hooks (span IDs, time, alloc stats) feeding telemetry. (Owner: copilot, 2026-03-10) → kernel-c/src/trace.c, kernel-c/include/infinity/trace.h
- [x] Implement crash-only restart semantics for kernel services (with state recovery policy). (Owner: copilot, 2026-03-10) → kernel-c/src/service_registry.c (ify_restart_policy_t + retry loop)

## D. Data Archiving and Storage
- [-] Define archival policies (hot/warm/cold tiers) per dimension/project. (Owner: copilot, 2026-03-10) → data/README.md
- [-] Define retention settings and legal holds (per artifact class). (Owner: copilot, 2026-03-10) → data/README.md
- [-] Interpret databases as multimedia with schema automations, retention, and circular analysis for DeFi resource pooling and circuit-based distribution. (Owner: copilot, 2026-03-10) → data/README.md
- [-] Implement storage adapters (local fs, object store, db-backed, IPFS for legal/regulatory artifacts) with uniform API. (Owner: copilot, 2026-03-10) → data/README.md
- [-] Implement integrity verification (hash chains, signatures optional) and restoration checks. (Owner: copilot, 2026-03-10) → data/README.md
- [-] Add provenance linking (artifact → producing task → agent → controller → node graph). (Owner: copilot, 2026-03-10) → data/README.md
- [-] Implement dataset versioning (semantic tags + immutable snapshots). (Owner: copilot, 2026-03-10) → data/README.md
- [-] Implement backup/restore tooling (CLI + in-app) with dry-run. (Owner: copilot, 2026-03-10) → data/README.md
- [-] Add deduplication strategy (content-addressed chunks) for large artifacts. (Owner: copilot, 2026-03-10) → data/README.md
- [-] Add encryption-at-rest + key management hooks. (Owner: copilot, 2026-03-10) → data/README.md
- [-] Add performance benchmarks for ingest/query/restore. (Owner: copilot, 2026-03-10) → data/README.md

## E. Editor Snippet Execution
- [-] Implement secure snippet runtime entrypoint from canvas nodes. (Owner: copilot, 2026-03-10)
- [-] Define snippet packaging format (code, deps, permissions, metadata). (Owner: copilot, 2026-03-10)
- [-] Add execution profiles (local, isolated, deployment-bound) with explicit boundaries. (Owner: copilot, 2026-03-10)
- [-] Implement permission/capability model (fs/net/model access) with prompts. (Owner: copilot, 2026-03-10)
- [-] Add deterministic execution mode (seeded, pinned deps) for reproducibility. (Owner: copilot, 2026-03-10)
- [-] Implement interpreter attachment API for editors (language servers + runtimes). (Owner: copilot, 2026-03-10)
- [-] Add sandbox escape tests and hardening checks. (Owner: copilot, 2026-03-10)
- [-] Implement output artifact capture (logs, files, structured results). (Owner: copilot, 2026-03-10)
- [-] Add resource limits (cpu/mem/time) and preflight validation. (Owner: copilot, 2026-03-10)
- [-] Add snippet-to-node compiler (turn editor code into reusable node templates). (Owner: copilot, 2026-03-10)

## F. Flow Graph and Node Connectivity
- [ ] Implement node/link/group graph data model with typed ports.
- [ ] Add deterministic serialization/deserialization (stable ordering + schema versioning).
- [ ] Implement cycle detection and execution-order planning.
- [ ] Implement node execution contracts (start/progress/complete/fail/cancel).
- [-] Add advanced flow control engine (conditional/loop/fallback + ML score gating). (Owner: copilot, 2026-03-11) → runtime-rust/crates/ify-controller/src/flow_control.rs
- [ ] Add graph diff/patch system (for collaborative and agent edits).
- [ ] Add graph validation (type compatibility, missing params, forbidden edges).
- [ ] Add subgraphs/macros (reuse, parameterization, versioning).
- [ ] Add node provenance (who/what/when/why) via ActionLog.
- [ ] Add connectors for workflow nodes (HTTP, blockchain, db, ML, trading).
- [ ] Add test harness for graph execution determinism.

## G. Governance and Policies
- [x] Define contribution and code ownership policy (CODEOWNERS + review rules). ([docs/governance/contribution-and-code-ownership.md](docs/governance/contribution-and-code-ownership.md), [CODEOWNERS](CODEOWNERS))
- [x] Define release gates for kernel/runtime/data changes. ([docs/governance/release-gates.md](docs/governance/release-gates.md))
- [x] Add policy checks for interface compatibility (CI). ([docs/governance/interface-compatibility-policy.md](docs/governance/interface-compatibility-policy.md))
- [x] Define security policy for agents/tools (allowed capabilities by tier). ([docs/governance/agent-security-policy.md](docs/governance/agent-security-policy.md))
- [x] Add audit requirements for privileged tasks (signing + retention). ([docs/governance/audit-policy.md](docs/governance/audit-policy.md))
- [x] Add dependency policy (licenses, vulnerability scanning). ([docs/governance/dependency-policy.md](docs/governance/dependency-policy.md))
- [x] Define model governance (which models allowed, evaluation requirements). ([docs/governance/model-governance.md](docs/governance/model-governance.md))
- [x] Add compliance checklist for marketplace submissions. ([docs/governance/marketplace-compliance.md](docs/governance/marketplace-compliance.md))
- [x] Add change-management policy for node schemas. ([docs/governance/schema-change-policy.md](docs/governance/schema-change-policy.md))
- [x] Define incident response process for agent/tool compromise. ([docs/governance/incident-response.md](docs/governance/incident-response.md))

## H. Hyperperformance Optimization
- [x] Add performance-driven hyperparameter tuning with quick reload triggers.
- [x] Add kernel-style replica pooling for multi-model (ML + AI) execution stacks.
- [ ] Set baseline performance budgets (latency/throughput/memory) per subsystem.
- [ ] Add profiling hooks to critical execution paths.
- [ ] Implement benchmark suite for kernel/runtime/canvas/graph.
- [ ] Add load testing for mesh artifact updates and node batching.
- [ ] Optimize scheduler hot paths (lock contention, queue ops).
- [ ] Optimize serialization/deserialization (zero-copy where feasible).
- [ ] Add caching strategy (artifact cache, node results cache).
- [ ] Add adaptive batching policies (backpressure-aware).
- [ ] Document optimization loops and measurable gains.
- [ ] Add regression guardrails (perf CI thresholds).

## I. Infinity Zoom Canvas UX Contracts
- [ ] Define zoom-level interaction contracts and limits.
- [ ] Implement node visibility + detail scaling policy.
- [ ] Add accessibility and navigation keyboard support.
- [ ] Implement multi-select, lasso, snap-to-grid, align/distribute.
- [ ] Add node inspector panel (params, tools, memory, logs, artifacts).
- [ ] Implement canvas search/command palette (nodes, agents, tasks).
- [ ] Add minimap + breadcrumbs + focus mode.
- [ ] Add collaborative cursors and edit conflict resolution (optional).
- [ ] Integrate seamless node adder/customizer from editor.
- [ ] Implement canvas performance budgets (FPS + large graph handling).
Implementation plan and highlights: [`docs/architecture/epic-i-implementation.md`](docs/architecture/epic-i-implementation.md)

## J. Job Scheduling and Task Lifecycle
- [ ] Define task states (queued/running/paused/failed/completed).
- [ ] Implement priority-aware scheduling.
- [ ] Add retries, backoff, and cancellation semantics.
- [ ] Implement task leasing/heartbeats for distributed workers.
- [ ] Implement per-dimension quotas and rate limiting.
- [ ] Add dependency-aware scheduling (DAG-based).
- [ ] Implement task preemption policy.
- [ ] Add task persistence + recovery after crash/restart.
- [ ] Add task templates (for repeated workflows).
- [ ] Add unique TaskID enforcement + index across dimensions.

## K. Kaizen Reliability Loop
- [ ] Define weekly reliability review cadence.
- [ ] Track MTTR/error budget/regression rate metrics.
- [ ] Apply one measurable reliability improvement per cycle.
- [ ] Implement SLOs for task execution and UI responsiveness.
- [ ] Add chaos testing for replication kernel and orchestrator.
- [ ] Add runbooks for common failures.
- [ ] Add automated incident creation from telemetry signals.
- [ ] Add regression triage workflow (labels, owners, SLA).
- [ ] Add postmortem template + publishing workflow.
- [ ] Add reliability dashboard integrated in widgets.

## L. Layered Module Interfaces
- [ ] Publish IDL/spec for cross-layer APIs.
- [ ] Add compatibility tests for interface evolution.
- [ ] Enforce semver rules for public contracts.
- [ ] Define stable event bus API (ActionLog + orchestration events).
- [ ] Define mesh artifact API (read/write/subscribe) across layers.
- [ ] Define node execution API (planner → executor → reporter).
- [ ] Define editor integration API (interpreter attach, LSP, runtimes).
- [ ] Add deprecation policy and migration tooling.
- [ ] Add API conformance test suite.
- [ ] Add reference implementations for key interfaces.

## M. Mesh Data Canvas
- [ ] Implement mesh data representation and routing.
- [ ] Add high-volume node update batching.
- [ ] Validate consistency under concurrent edits.
- [ ] Define mesh artifact schema registry + versions.
- [ ] Implement mesh subscriptions (watch nodes/artifacts) with filters.
- [ ] Add conflict resolution strategy (OT/CRDT or patch merge).
- [ ] Add provenance stamping for each mesh write.
- [ ] Implement artifact indexing (search by tags, TaskID, node, agent).
- [ ] Add garbage collection for orphaned artifacts.
- [ ] Add mesh replication between runtime instances.

## N. Node Instance Grouping
- [ ] Implement instance templates from grouped nodes.
- [ ] Add clone/fork mechanics with provenance tracking.
- [ ] Support instance-level configuration overrides.
- [ ] Add parameter inheritance rules.
- [ ] Add template versioning + migration.
- [ ] Add sharing/export of templates.
- [ ] Add locking policy (read-only templates vs editable).
- [ ] Add marketplace publishing hooks.
- [ ] Add test coverage for template expansion determinism.
- [ ] Add UI for managing templates in project window.

## O. Operational Security
- [ ] Threat-model desktop-to-canvas execution path.
- [ ] Add input validation at all boundary layers.
- [ ] Add audit trail for privileged actions.
- [ ] Implement identity-first access controls (users/agents/tools).
- [ ] Add signed artifacts for runtime/deploy paths.
- [ ] Implement sandboxed tool execution (network/fs/model boundaries).
- [ ] Add secret management and redaction.
- [ ] Add supply chain protections (SBOM, signature verification).
- [ ] Add policy engine for allow/deny decisions.
- [ ] Add security hardening checklist before GA.

## P. Processing and Transformation Pipelines
- [ ] Build pipeline primitives (map/filter/reduce/aggregate/window).
- [ ] Add transform versioning and replay.
- [ ] Add dead-letter handling for failed transforms.
- [ ] Add schema inference and validation for datasets.
- [ ] Add streaming mode + watermarking.
- [ ] Add checkpointing and resume.
- [ ] Add lineage tracking integrated with mesh artifacts.
- [ ] Add performance optimization for large transforms.
- [ ] Add UI pipeline builder nodes.
- [ ] Add connectors to DB and object storage.

## Q. Quality Engineering
- [ ] Add unit/integration/performance test strategy.
- [ ] Add deterministic test datasets for graph/data paths.
- [ ] Set quality gates for merge readiness.
- [ ] Add fuzz testing for parsers/serializers.
- [ ] Add security testing (SAST/DAST) pipeline.
- [ ] Add contract tests for interfaces (IDL).
- [ ] Add golden tests for UI layouts.
- [ ] Add load tests for orchestrator and mesh.
- [ ] Add test reporting widgets.
- [ ] Add release candidate validation checklist.

## R. Rust Performer Runtime
- [ ] Scaffold runtime crates and workspace.
- [ ] Implement executor for agentic combo ML tasks.
- [ ] Add safe FFI boundary wrappers for kernel calls.
- [ ] Implement tool runner abstraction (db/http/blockchain/model).
- [ ] Implement memory subsystem (short/long-term, vector store hook).
- [ ] Add planner integration (plans → tasks → nodes).
- [ ] Add cancellation and cooperative yield.
- [ ] Add sandbox integration (capabilities passed from kernel).
- [ ] Add structured logging + traces.
- [ ] Add conformance tests against kernel ABI.

## S. Snippet and Agent Marketplace Foundations
- [ ] Define package format for reusable snippets/agents.
- [ ] Add signature verification and trust policy.
- [ ] Add dependency compatibility checks.
- [ ] Implement marketplace registry (local + remote).
- [ ] Add publishing workflow (lint, scan, sign, upload).
- [ ] Add sandboxing requirements for marketplace content.
- [ ] Add ratings/metadata taxonomy (capabilities, costs, domains).
- [ ] Add compatibility certification tests.
- [ ] Add rollback/disable mechanism for compromised packages.
- [ ] Add UI surfaces in project window for marketplace.

## T. Telemetry and Observability
- [ ] Define logs/metrics/traces taxonomy.
- [ ] Implement distributed tracing across layers.
- [ ] Build dashboard views for runtime health.
- [ ] Add per-task timelines (planner → controllers → nodes → artifacts).
- [ ] Add per-node timing and resource overlays in canvas.
- [ ] Add alerting rules (SLO violations, error spikes).
- [ ] Add cost accounting (tokens, cpu, gpu) by TaskID/dimension.
- [ ] Add privacy controls for telemetry.
- [ ] Add debug capture bundles for bug reports.
- [ ] Add observability widgets in UI.

## U. Upgrade and Migration System
- [ ] Implement versioned migration framework.
- [ ] Add rollback-safe data and interface migrations.
- [ ] Document zero-downtime upgrade playbooks.
- [ ] Add schema migrations for mesh artifacts.
- [ ] Add node graph migrations and compatibility mapping.
- [ ] Add kernel ABI version negotiation.
- [ ] Add runtime feature-flag framework.
- [ ] Add migration test harness.
- [ ] Add upgrade UI flows and status.
- [ ] Add downgrade restrictions policy.

## V. Visualization and Debugging Tools
- [ ] Add runtime graph inspector and execution playback.
- [ ] Add per-node timing and resource overlays.
- [ ] Add failure provenance and root-cause mapping.
- [ ] Add ActionLog viewer (filter by TaskID/dimension/node).
- [ ] Add artifact diff viewer (before/after transforms).
- [ ] Add live orchestrator queue viewer.
- [ ] Add breakpoint/step execution for nodes.
- [ ] Add editor-to-node trace (what code produced what node/action).
- [ ] Add debugging widgets (logs, traces, metrics) dockable.
- [ ] Add exportable debug sessions.

## W. Workload Distribution and Deployment
- [ ] Implement distributed execution planner.
- [ ] Add deployment targets (local, cluster, hybrid).
- [ ] Validate robustness under partial node failures.
- [ ] Add worker registration + capability matching.
- [ ] Add artifact transport between workers.
- [ ] Add autoscaling hooks (based on queues and SLOs).
- [ ] Add multi-tenant isolation boundaries.
- [ ] Add canary deployments for runtime.
- [ ] Add disaster recovery plan.
- [ ] Add deployment UI workflows.

## X. eXternal Integrations
- [ ] Define integration SDK/API for external tools.
- [ ] Add sandboxed adapter model for third-party connectors.
- [ ] Add compatibility certification tests.
- [ ] Add HTTP request node (REST/GraphQL/webhooks) with auth and retries.
- [ ] Add blockchain nodes (wallet, signing, RPC, events).
- [ ] Add database connector nodes (read/write/stream).
- [ ] Add library manager integration (deps, version pinning).
- [ ] Add workflow composer for connecting integrations to blockControllers.
- [ ] Add integration secrets handling.
- [ ] Add end-to-end integration test suite.

## Y. Yield and Capacity Planning
- [ ] Model capacity envelopes for compute/storage/network.
- [ ] Add autoscaling policies by workload class.
- [ ] Build saturation alerts and mitigation actions.
- [ ] Add per-dimension quotas and budgets.
- [ ] Add forecasting from historical telemetry.
- [ ] Add cost estimation for plans before execution.
- [ ] Add capacity reservation for high-priority tasks.
- [ ] Add GPU scheduling strategy for ML workloads.
- [ ] Add rate limiting for integrations.
- [ ] Add reporting dashboard in widgets.

## Z. Zero-Trust Finalization
- [ ] Enforce identity-first access controls.
- [ ] Require signed artifacts for runtime/deploy paths.
- [ ] Complete security hardening checklist before GA.
- [ ] Add continuous policy evaluation in orchestrator.
- [ ] Add attestation for agent/tool binaries and packages.
- [ ] Add secure-by-default network egress rules.
- [ ] Add least-privilege templates for common workflows.
- [ ] Add red-team exercises and fixes.
- [ ] Add GA readiness review gates.
- [ ] Add final security documentation bundle.
