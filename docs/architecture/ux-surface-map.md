# Product UX Surface Map

**Status:** `[-]` in progress  
**Epic:** A — Architecture Foundation  
**Owner:** Architecture team / Canvas team

---

## 1. Purpose

This document defines the **product UX surface map** for infinityOS — the complete set of UI panels, windows, and interaction surfaces, their responsibilities, and their integration points with the runtime layer.

---

## 2. Surface Inventory

```
┌─────────────────────────────────────────────────────────────────────┐
│  infinityOS Application Window                                      │
│                                                                     │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │  Command Palette / Global Search          (overlay on demand) │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
│  ┌─────────────┐  ┌───────────────────────────────┐  ┌──────────┐  │
│  │  Project    │  │                               │  │  Chat    │  │
│  │  Window     │  │   Infinity Zoom Canvas        │  │  Column  │  │
│  │             │  │                               │  │          │  │
│  │  (sidebar)  │  │   (primary workspace)         │  │ (agent   │  │
│  │             │  │                               │  │  panel)  │  │
│  └─────────────┘  └───────────────────────────────┘  └──────────┘  │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────────┐│
│  │  Status Bar / Widgets Row                                       ││
│  └─────────────────────────────────────────────────────────────────┘│
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────────┐│
│  │  Multiplex Agents Window          (dockable / floating)         ││
│  └─────────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────────┘
```

---

## 3. Surface Definitions

### 3.1 Infinity Zoom Canvas

**Purpose:** Primary workspace.  An infinite, zoomable canvas where nodes, links, and groups represent code, data pipelines, agents, and deployment units.

**Responsibilities:**
- Render and manage the node graph at arbitrary zoom levels.
- Handle multi-select, lasso, snap-to-grid, align/distribute operations.
- Provide inline node creation (seamless node adder) and node customization.
- Surface node execution state (queued/running/failed/completed) visually.
- Integrate the minimap, breadcrumbs, and focus mode for navigation.

**Runtime Integration:**
- Subscribes to mesh artifact updates via the Performer Runtime subscription API.
- Emits `node.created`, `node.updated`, `node.linked` ActionLog events on every graph mutation.
- Dispatches `task.submitted` events when the user triggers node execution.

---

### 3.2 Project Window

**Purpose:** Left sidebar providing project-level navigation, asset management, and settings.

**Sections:**
- **Files / Assets**: project files, imported datasets, exported artifacts.
- **Agents**: list of active and template agents in the current dimension.
- **Templates**: reusable node groups and instance templates (EPIC N).
- **Marketplace**: browse and install snippets/agents from the marketplace (EPIC S).
- **Settings**: dimension configuration, capability profile, deployment targets.

**Runtime Integration:**
- Reads dimension metadata and capability profile from the Performer Runtime.
- Triggers `dimension.created` flows when a new project is initialized.

---

### 3.3 Chat Column

**Purpose:** Right sidebar providing a conversational interface to agents and the system.

**Responsibilities:**
- Display agent conversation history (user messages + agent responses).
- Show inline artifact previews (code, images, structured data).
- Surface agent plan steps with expandable detail.
- Allow users to inspect, accept, reject, or replay agent actions.

**Runtime Integration:**
- Bound to an agent instance via the Performer Runtime agent API.
- Every message exchange is captured as `agent.tool_called` / `agent.tool_result` ActionLog entries.
- Correlation IDs link all downstream tasks to the originating user message.

---

### 3.4 Multiplex Agents Window

**Purpose:** Dockable panel for managing multiple concurrent agent sessions.

**Responsibilities:**
- List all active agent sessions across dimensions.
- Show per-agent status (idle, planning, executing, waiting).
- Allow users to pause, resume, and cancel agent sessions.
- Display resource consumption (tokens used, tasks spawned, artifacts produced).

**Runtime Integration:**
- Polls agent session state from the Performer Runtime.
- Emits `agent.plan_generated` and `agent.evaluated` events as agents progress.

---

### 3.5 Widgets Row (Status Bar)

**Purpose:** Persistent bottom bar providing system health, resource usage, and quick actions.

**Widgets (initial set):**
- **Dimension indicator**: current dimension name and tier.
- **Task queue gauge**: queued / running / completed counts.
- **Memory usage**: kernel-reported arena stats.
- **Capability indicator**: active capability profile icon.
- **SLO status**: green/amber/red based on current error budget (EPIC K).

---

### 3.6 Node Inspector Panel

**Purpose:** Context-sensitive panel that surfaces detail for the selected node.

**Tabs:**
- **Parameters**: editable node configuration.
- **Tools**: tools available to this node's agent (with capability indicators).
- **Memory**: short/long-term memory slots wired to this node.
- **Logs**: recent ActionLog entries scoped to this node's TaskIDs.
- **Artifacts**: artifacts produced/consumed by this node.

---

### 3.7 Command Palette / Global Search

**Purpose:** Overlay triggered by `Cmd/Ctrl+K` for fast navigation and action dispatch.

**Capabilities:**
- Search nodes, agents, artifacts, and tasks by name or ID.
- Execute global actions (create node, run selection, deploy, open settings).
- Filter by dimension, time range, task state.

---

## 4. Navigation and Zoom Model

The infinity zoom canvas supports five conceptual zoom levels:

| Level | Name | What's Visible |
|-------|------|----------------|
| 5 | **Galaxy** | Dimension clusters and project-level groups. |
| 4 | **Constellation** | Named node groups and instance templates. |
| 3 | **Node** | Individual nodes with port labels (default working level). |
| 2 | **Detail** | Node internals: code snippets, parameter editors. |
| 1 | **Micro** | Byte-level diff views, performance overlays, artifact content. |

Zoom transitions are animated at ≥ 60 FPS.  Node detail scales with zoom level; invisible elements are culled.

---

## 5. References

- [`reference-flow.md`](reference-flow.md) — End-to-end user request flow from UX to execution.
- EPIC F — Flow Graph and Node Connectivity.
- EPIC I — Infinity Zoom Canvas UX Contracts.
- EPIC B — blockControllerGenerator Regime (node execution integration).
