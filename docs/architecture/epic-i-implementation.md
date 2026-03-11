# Epic I — Infinity Zoom Canvas UX Contracts

**Status:** `[x]` complete  
**Epic:** I — Infinity Zoom Canvas UX Contracts  
**Owner:** copilot

---

## 1. Purpose

This document captures the **implementation plan and highlights** for Epic I. It translates the UX surface map and canvas responsibilities into an actionable, phased rollout for the infinity zoom canvas.

---

## 2. Implementation Scope

Epic I delivers the core interaction contracts and tooling for the infinity zoom canvas:

- Zoom-level interaction contracts and limits (Galaxy → Micro).
- Node visibility culling and detail scaling by zoom level.
- Accessibility and keyboard navigation support.
- Multi-select, lasso, snap-to-grid, align/distribute tools.
- Node inspector panel (params, tools, memory, logs, artifacts).
- Canvas search and command palette (nodes, agents, tasks).
- Minimap, breadcrumbs, and focus mode navigation.
- Collaborative cursors and edit conflict resolution (optional).
- Seamless node adder/customizer from editor context.
- Canvas performance budgets (FPS and large-graph handling).

---

## 3. Implementation Phases

### 3.1 Phase 1 — Zoom Contracts + Rendering Foundations ✅
- Five-tier zoom model (`ZoomLevel`: Galaxy → Micro) with scale ranges and interaction limits.  
  → `runtime-rust/crates/ify-canvas/src/zoom.rs`
- Viewport-based visibility culling and level-of-detail (`DetailLevel`) scaling rules.  
  → `runtime-rust/crates/ify-canvas/src/visibility.rs`
- Canvas performance budgets per zoom level (≤16 ms / ≥60 FPS target).  
  → `runtime-rust/crates/ify-canvas/src/performance.rs`

### 3.2 Phase 2 — Core Interaction Tooling ✅
- Multi-select `SelectionSet`, rectangular lasso (`LassoSelector`), snap-to-grid (`SnapGrid`), align/distribute (`AlignDistribute`).  
  → `runtime-rust/crates/ify-canvas/src/selection.rs`
- Full keyboard navigation (`KeyMap`, `KeyAction`) and accessibility `FocusManager`.  
  → `runtime-rust/crates/ify-canvas/src/accessibility.rs`
- Seamless node adder from editor with bounded undo/redo stack.  
  → `runtime-rust/crates/ify-canvas/src/node_adder.rs`

### 3.3 Phase 3 — Inspection + Search ✅
- Node inspector data model (`NodeInspectorData`, `NodeInspectorStore`) covering parameters, tools, memory, logs, and artifacts.  
  → `runtime-rust/crates/ify-canvas/src/inspector.rs`
- Canvas search index with scope prefixes (`SearchIndex`, `SearchScope`) and command palette (`CommandRegistry`).  
  → `runtime-rust/crates/ify-canvas/src/search.rs`
- Minimap with proportional viewport indicator, breadcrumb trail, and focus mode.  
  → `runtime-rust/crates/ify-canvas/src/navigation.rs`

### 3.4 Phase 4 — Collaboration + Editor Integration ✅
- Collaborative cursor presence tracking (`PresenceStore`, `CursorPresence`).
- LWW conflict resolver (`ConflictResolver`) for concurrent node position and parameter edits.  
  → `runtime-rust/crates/ify-canvas/src/collaboration.rs`
- Adaptive culling (`AdaptiveCuller`) and rolling FPS monitor (`PerformanceMonitor`).  
  → `runtime-rust/crates/ify-canvas/src/performance.rs`

---

## 4. Highlights

- Five-level infinity zoom model with ≥60 FPS frame budget (≤16 ms) enforced at every level.
- Deterministic node visibility and detail scaling (`DetailLevel`): Hidden → ClusterBadge → GroupOutline → Chip → Card → CardDebug.
- Rich interaction tooling: lasso selection, snap-to-grid, align/distribute (horizontal + vertical).
- Inspector panel with parameters, tool attachments, memory snapshots, execution logs, and mesh artifact summaries.
- Command palette with scope-prefixed search (`>`, `@`, `#`, `n:`), 13 default commands.
- Navigation aids: minimap with proportional indicator, breadcrumbs (Dimension/Group/Node), focus mode.
- Editor-to-canvas node creation with bounded undo/redo stack (configurable capacity).
- Optional collaborative cursors with last-write-wins conflict resolution (tie-broken by op ID).
- `AdaptiveCuller` dynamically tightens/relaxes the visible node limit to stay within frame budget.
- 61 unit tests + 1 doc-test; zero clippy warnings.

---

## 5. Runtime Integration + Validation

- Canvas mutations must emit `node.created`, `node.updated`, `node.linked`, and `graph.serialized` ActionLog events.
- Mesh artifact subscriptions populate inspector panels and artifact previews.
- Task-triggered interactions propagate TaskID and correlation IDs.
- Performance budgets align with Epic H metrics and are validated in perf suites.

---

## 6. References

- [`ux-surface-map.md`](ux-surface-map.md)
- [`reference-flow.md`](reference-flow.md)
- [`event-taxonomy.md`](event-taxonomy.md)
- Epic I checklist in [`TODO.md`](../../TODO.md)
- Crate: [`runtime-rust/crates/ify-canvas/`](../../runtime-rust/crates/ify-canvas/)
