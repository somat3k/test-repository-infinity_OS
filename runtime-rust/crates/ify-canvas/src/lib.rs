//! # ify-canvas — Infinity Zoom Canvas UX Contracts
//!
//! This crate implements the full **EPIC I** feature set for infinityOS.
//! It delivers the core interaction contracts, data models, and state machines
//! for the infinity zoom canvas layer.
//!
//! ## Module map
//!
//! | Module            | Epic I item                                         |
//! |-------------------|-----------------------------------------------------|
//! | [`zoom`]          | Zoom-level interaction contracts and limits         |
//! | [`visibility`]    | Node visibility + detail scaling policy             |
//! | [`accessibility`] | Accessibility and keyboard navigation support       |
//! | [`selection`]     | Multi-select, lasso, snap-to-grid, align/distribute |
//! | [`inspector`]     | Node inspector panel (params, tools, memory, logs)  |
//! | [`search`]        | Canvas search and command palette                   |
//! | [`navigation`]    | Minimap, breadcrumbs, and focus mode                |
//! | [`collaboration`] | Collaborative cursors and edit conflict resolution  |
//! | [`node_adder`]    | Seamless node adder/customizer from editor          |
//! | [`performance`]   | Canvas performance budgets (FPS + large-graph)      |
//!
//! ## Quick start
//!
//! ```rust
//! use ify_canvas::{
//!     zoom::{ZoomLevel, ZoomState},
//!     visibility::VisibilityPolicy,
//!     accessibility::{KeyMap, KeyBinding, FocusManager},
//!     selection::{SelectionSet, SnapGrid},
//!     performance::PerformanceBudget,
//! };
//!
//! // Set up a canvas viewport at standard zoom.
//! let mut state = ZoomState::new();
//! assert_eq!(state.level(), ZoomLevel::Standard);
//!
//! // Zoom in toward micro level.
//! state.zoom(2.5, (0.0, 0.0)).unwrap();
//!
//! // Determine what detail level a node should render at.
//! let policy = VisibilityPolicy::new(state.level(), (0.0, 0.0, 800.0, 600.0));
//! let detail = policy.detail_for_node((10.0, 10.0, 100.0, 80.0));
//! assert!(detail.is_visible());
//!
//! // Keyboard navigation.
//! let km = KeyMap::default_canvas();
//! assert!(km.resolve(&KeyBinding::plain("+")).is_some());
//!
//! // Selection.
//! let mut sel = SelectionSet::new();
//! sel.add("node-1");
//! assert!(sel.contains("node-1"));
//!
//! // Performance budget for the current zoom.
//! let budget = PerformanceBudget::for_zoom(state.level());
//! assert!(budget.frame_budget_ms <= 16.0);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod accessibility;
pub mod collaboration;
pub mod inspector;
pub mod navigation;
pub mod node_adder;
pub mod performance;
pub mod search;
pub mod selection;
pub mod visibility;
pub mod zoom;

// Re-export the most commonly used types at the crate root.
pub use accessibility::{AccessibilityError, FocusManager, KeyAction, KeyBinding, KeyMap};
pub use collaboration::{
    CollaboratorId, ConflictResolver, CursorPresence, EditOperation, NodeChange, PresenceStore,
};
pub use inspector::{
    ArtifactSummary, LogEntry, LogLevel, MemorySnapshot, NodeInspectorData, NodeInspectorStore,
    Parameter, ParameterValue, ToolAttachment,
};
pub use navigation::{Breadcrumbs, BreadcrumbEntry, BreadcrumbKind, FocusMode, Minimap, NavigationState};
pub use node_adder::{
    AddNodeRequest, AddNodeResult, CanvasNodeTemplate, NodeAdder, NodeAdderError, UndoEntry,
    UndoStack,
};
pub use performance::{AdaptiveCuller, FrameSample, PerformanceBudget, PerformanceMonitor};
pub use search::{
    CanvasCommand, CommandRegistry, SearchIndex, SearchItem, SearchItemKind, SearchScope,
};
pub use selection::{
    AlignAnchor, AlignDistribute, AlignError, Axis, LassoSelector, Point, Rect, SelectionSet,
    SnapGrid,
};
pub use visibility::{DetailLevel, VisibilityPolicy};
pub use zoom::{ZoomConstraints, ZoomError, ZoomLevel, ZoomState, FRAME_BUDGET_MS, SCALE_MAX, SCALE_MIN};
