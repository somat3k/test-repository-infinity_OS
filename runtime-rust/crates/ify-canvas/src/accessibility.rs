//! # accessibility — Accessibility and Keyboard Navigation
//!
//! Provides keyboard navigation contracts and accessibility affordances for the
//! infinity canvas.  All interactions that can be performed with a pointing
//! device must also be achievable via keyboard commands defined here.
//!
//! ## Key bindings (default)
//!
//! | Key / chord              | Action                                    |
//! |--------------------------|-------------------------------------------|
//! | `+` / `=`                | Zoom in one step                          |
//! | `-`                      | Zoom out one step                         |
//! | `0`                      | Reset zoom to Standard (1.0)              |
//! | Arrow keys               | Pan the viewport                          |
//! | `Tab`                    | Cycle focus to the next node              |
//! | `Shift+Tab`              | Cycle focus to the previous node          |
//! | `Space`                  | Toggle selection on focused node          |
//! | `Escape`                 | Clear selection / close overlay           |
//! | `Enter`                  | Open inspector for focused node           |
//! | `Ctrl+A`                 | Select all nodes                          |
//! | `Ctrl+F` / `Ctrl+/`      | Open command palette                      |
//! | `Ctrl+Z`                 | Undo last structural edit                 |
//! | `Ctrl+Shift+Z` / `Ctrl+Y`| Redo                                      |
//! | `Delete` / `Backspace`   | Delete selected nodes/edges               |
//! | `F` (with selection)     | Fit selection into viewport               |
//! | `M`                      | Toggle minimap visibility                 |

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// KeyAction
// ---------------------------------------------------------------------------

/// A high-level canvas action that can be triggered by a keyboard event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyAction {
    /// Zoom in one discrete step.
    ZoomIn,
    /// Zoom out one discrete step.
    ZoomOut,
    /// Reset zoom to Standard level (scale 1.0).
    ZoomReset,
    /// Pan the viewport left.
    PanLeft,
    /// Pan the viewport right.
    PanRight,
    /// Pan the viewport up.
    PanUp,
    /// Pan the viewport down.
    PanDown,
    /// Move keyboard focus to the next node.
    FocusNext,
    /// Move keyboard focus to the previous node.
    FocusPrevious,
    /// Toggle selection on the focused node.
    ToggleSelection,
    /// Clear the current selection or close any open overlay.
    ClearOrClose,
    /// Open the node inspector for the focused node.
    OpenInspector,
    /// Select all nodes in the current viewport.
    SelectAll,
    /// Open the command palette / canvas search.
    OpenCommandPalette,
    /// Undo the last structural canvas edit.
    Undo,
    /// Redo the previously undone edit.
    Redo,
    /// Delete the selected nodes and/or edges.
    DeleteSelected,
    /// Fit selected nodes into the viewport.
    FitSelection,
    /// Toggle minimap visibility.
    ToggleMinimap,
}

// ---------------------------------------------------------------------------
// KeyBinding
// ---------------------------------------------------------------------------

/// A keyboard binding descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyBinding {
    /// Primary key identifier (e.g. `"+"`, `"ArrowLeft"`, `"a"`).
    pub key: String,
    /// Whether the Control / Meta modifier must be held.
    pub ctrl: bool,
    /// Whether the Shift modifier must be held.
    pub shift: bool,
    /// Whether the Alt modifier must be held.
    pub alt: bool,
}

impl KeyBinding {
    /// Create a simple unmodified binding.
    pub fn plain(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    /// Create a Ctrl+key binding.
    pub fn ctrl(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ctrl: true,
            shift: false,
            alt: false,
        }
    }

    /// Create a Ctrl+Shift+key binding.
    pub fn ctrl_shift(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ctrl: true,
            shift: true,
            alt: false,
        }
    }
}

// ---------------------------------------------------------------------------
// KeyMap
// ---------------------------------------------------------------------------

/// A map from [`KeyBinding`] to [`KeyAction`].
///
/// The default keymap implements the bindings described in the module docs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMap {
    bindings: HashMap<KeyBinding, KeyAction>,
}

impl KeyMap {
    /// Create a keymap with the default canvas bindings.
    pub fn default_canvas() -> Self {
        let mut m = HashMap::new();

        // Zoom
        m.insert(KeyBinding::plain("+"), KeyAction::ZoomIn);
        m.insert(KeyBinding::plain("="), KeyAction::ZoomIn);
        m.insert(KeyBinding::plain("-"), KeyAction::ZoomOut);
        m.insert(KeyBinding::plain("0"), KeyAction::ZoomReset);

        // Pan
        m.insert(KeyBinding::plain("ArrowLeft"), KeyAction::PanLeft);
        m.insert(KeyBinding::plain("ArrowRight"), KeyAction::PanRight);
        m.insert(KeyBinding::plain("ArrowUp"), KeyAction::PanUp);
        m.insert(KeyBinding::plain("ArrowDown"), KeyAction::PanDown);

        // Focus
        m.insert(KeyBinding::plain("Tab"), KeyAction::FocusNext);
        m.insert(
            KeyBinding {
                key: "Tab".into(),
                ctrl: false,
                shift: true,
                alt: false,
            },
            KeyAction::FocusPrevious,
        );

        // Selection
        m.insert(KeyBinding::plain("Space"), KeyAction::ToggleSelection);
        m.insert(KeyBinding::plain("Escape"), KeyAction::ClearOrClose);
        m.insert(KeyBinding::plain("Enter"), KeyAction::OpenInspector);
        m.insert(KeyBinding::ctrl("a"), KeyAction::SelectAll);

        // Command palette
        m.insert(KeyBinding::ctrl("f"), KeyAction::OpenCommandPalette);
        m.insert(KeyBinding::ctrl("/"), KeyAction::OpenCommandPalette);

        // Undo/redo
        m.insert(KeyBinding::ctrl("z"), KeyAction::Undo);
        m.insert(KeyBinding::ctrl_shift("z"), KeyAction::Redo);
        m.insert(KeyBinding::ctrl("y"), KeyAction::Redo);

        // Delete
        m.insert(KeyBinding::plain("Delete"), KeyAction::DeleteSelected);
        m.insert(KeyBinding::plain("Backspace"), KeyAction::DeleteSelected);

        // Navigation
        m.insert(KeyBinding::plain("f"), KeyAction::FitSelection);
        m.insert(KeyBinding::plain("m"), KeyAction::ToggleMinimap);

        Self { bindings: m }
    }

    /// Look up the action for a key event.
    ///
    /// Returns `None` if no binding matches.
    pub fn resolve(&self, binding: &KeyBinding) -> Option<KeyAction> {
        self.bindings.get(binding).copied()
    }

    /// Register a custom binding, overriding any existing one.
    pub fn register(&mut self, binding: KeyBinding, action: KeyAction) {
        self.bindings.insert(binding, action);
    }

    /// Remove a binding.
    pub fn remove(&mut self, binding: &KeyBinding) {
        self.bindings.remove(binding);
    }

    /// Return an iterator over all registered bindings.
    pub fn iter(&self) -> impl Iterator<Item = (&KeyBinding, &KeyAction)> {
        self.bindings.iter()
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::default_canvas()
    }
}

// ---------------------------------------------------------------------------
// AccessibilityError
// ---------------------------------------------------------------------------

/// Errors produced by accessibility operations.
#[derive(Debug, Error, PartialEq)]
pub enum AccessibilityError {
    /// The supplied key binding is already assigned to a different action.
    #[error("key binding already assigned to a different action")]
    BindingConflict,
}

// ---------------------------------------------------------------------------
// FocusManager
// ---------------------------------------------------------------------------

/// Tracks keyboard focus within the node graph.
///
/// Focus advances through nodes in the order they were registered.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FocusManager {
    /// Ordered list of focusable node IDs (as string handles).
    nodes: Vec<String>,
    /// Index of the currently focused node, if any.
    focused: Option<usize>,
}

impl FocusManager {
    /// Create an empty focus manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a focusable node.
    pub fn register_node(&mut self, node_id: impl Into<String>) {
        self.nodes.push(node_id.into());
    }

    /// Remove a node from the focus list.
    pub fn unregister_node(&mut self, node_id: &str) {
        if let Some(pos) = self.nodes.iter().position(|n| n == node_id) {
            self.nodes.remove(pos);
            match self.focused {
                Some(i) if i == pos => self.focused = None,
                Some(i) if i > pos => self.focused = Some(i - 1),
                _ => {}
            }
        }
    }

    /// Move focus to the next node, wrapping around.
    pub fn focus_next(&mut self) -> Option<&str> {
        if self.nodes.is_empty() {
            return None;
        }
        let next = match self.focused {
            None => 0,
            Some(i) => (i + 1) % self.nodes.len(),
        };
        self.focused = Some(next);
        self.nodes.get(next).map(String::as_str)
    }

    /// Move focus to the previous node, wrapping around.
    pub fn focus_previous(&mut self) -> Option<&str> {
        if self.nodes.is_empty() {
            return None;
        }
        let prev = match self.focused {
            None => self.nodes.len().saturating_sub(1),
            Some(0) => self.nodes.len() - 1,
            Some(i) => i - 1,
        };
        self.focused = Some(prev);
        self.nodes.get(prev).map(String::as_str)
    }

    /// Returns the currently focused node ID, if any.
    pub fn focused_node(&self) -> Option<&str> {
        self.focused
            .and_then(|i| self.nodes.get(i))
            .map(String::as_str)
    }

    /// Clear focus.
    pub fn clear_focus(&mut self) {
        self.focused = None;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_zoom_bindings() {
        let km = KeyMap::default_canvas();
        assert_eq!(
            km.resolve(&KeyBinding::plain("+")),
            Some(KeyAction::ZoomIn)
        );
        assert_eq!(
            km.resolve(&KeyBinding::plain("-")),
            Some(KeyAction::ZoomOut)
        );
        assert_eq!(
            km.resolve(&KeyBinding::plain("0")),
            Some(KeyAction::ZoomReset)
        );
    }

    #[test]
    fn default_keymap_undo_redo() {
        let km = KeyMap::default_canvas();
        assert_eq!(km.resolve(&KeyBinding::ctrl("z")), Some(KeyAction::Undo));
        assert_eq!(
            km.resolve(&KeyBinding::ctrl_shift("z")),
            Some(KeyAction::Redo)
        );
        assert_eq!(km.resolve(&KeyBinding::ctrl("y")), Some(KeyAction::Redo));
    }

    #[test]
    fn keymap_custom_binding() {
        let mut km = KeyMap::default_canvas();
        km.register(KeyBinding::plain("g"), KeyAction::ZoomReset);
        assert_eq!(
            km.resolve(&KeyBinding::plain("g")),
            Some(KeyAction::ZoomReset)
        );
    }

    #[test]
    fn focus_manager_cycle() {
        let mut fm = FocusManager::new();
        fm.register_node("a");
        fm.register_node("b");
        fm.register_node("c");

        assert_eq!(fm.focus_next(), Some("a"));
        assert_eq!(fm.focus_next(), Some("b"));
        assert_eq!(fm.focus_next(), Some("c"));
        // Wraps around.
        assert_eq!(fm.focus_next(), Some("a"));
    }

    #[test]
    fn focus_manager_previous_wraps() {
        let mut fm = FocusManager::new();
        fm.register_node("x");
        fm.register_node("y");

        assert_eq!(fm.focus_previous(), Some("y"));
        assert_eq!(fm.focus_previous(), Some("x"));
    }

    #[test]
    fn focus_manager_unregister_adjusts_index() {
        let mut fm = FocusManager::new();
        fm.register_node("a");
        fm.register_node("b");
        fm.register_node("c");
        fm.focus_next(); // focus = "a" (index 0)
        fm.unregister_node("b");
        // "a" is still focused at index 0.
        assert_eq!(fm.focused_node(), Some("a"));
    }

    #[test]
    fn focus_manager_empty_returns_none() {
        let mut fm = FocusManager::new();
        assert_eq!(fm.focus_next(), None);
        assert_eq!(fm.focus_previous(), None);
    }
}
