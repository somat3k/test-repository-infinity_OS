# canvas — Node Graph and Mesh Canvas Logic

The canvas layer owns the infinity zoom canvas UX contracts, the node/link/group/instance graph data model, and the desktop-level snippet execution interfaces.

## Responsibilities

- Node graph data model with typed ports, deterministic serialization, and schema versioning
- Infinity zoom interaction contracts (zoom levels, visibility, performance budgets)
- Multi-select, lasso, snap-to-grid, align/distribute, minimap, breadcrumbs
- Node inspector panel (params, tools, memory, logs, artifacts)
- Canvas search/command palette
- Seamless node adder and node customizer (from editor + user intent)
- Collaborative cursors and edit conflict resolution

## Constraints

- **Depends only on Performer Runtime**: no direct calls to kernel, data, deploy, or agents layers.
- **Stable node identity**: every node has a persistent, globally unique identity; references never silently break.
- **Deterministic serialization**: graph snapshots serialize with stable field ordering and schema version tags.
- **Clear user feedback**: every run/deploy/fail state surfaces a human-readable message.

## Epic Tracking

See [EPIC F — Flow Graph and Node Connectivity](../TODO.md) and [EPIC I — Infinity Zoom Canvas UX Contracts](../TODO.md) in `TODO.md`. Implementation details and highlights for Epic I live in [`docs/architecture/epic-i-implementation.md`](../docs/architecture/epic-i-implementation.md).
