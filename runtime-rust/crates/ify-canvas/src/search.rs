//! # search — Canvas Search and Command Palette
//!
//! Implements the canvas search overlay and command palette.  Users can
//! discover nodes, running agents, and tasks by typing free-text queries or
//! prefixed commands (e.g. `>` for commands, `@` for agents, `#` for tasks).
//!
//! ## Query prefix conventions
//!
//! | Prefix | Scope         |
//! |--------|---------------|
//! | (none) | All items     |
//! | `>`    | Commands only |
//! | `@`    | Agents only   |
//! | `#`    | Tasks only    |
//! | `n:`   | Nodes only    |

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SearchScope
// ---------------------------------------------------------------------------

/// The scope of a canvas search query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchScope {
    /// Search across all item kinds.
    All,
    /// Canvas commands only.
    Commands,
    /// Agents only.
    Agents,
    /// Tasks only.
    Tasks,
    /// Nodes only.
    Nodes,
}

impl SearchScope {
    /// Parse the scope from a query prefix.
    pub fn from_prefix(query: &str) -> (Self, &str) {
        if let Some(rest) = query.strip_prefix('>') {
            (SearchScope::Commands, rest.trim_start())
        } else if let Some(rest) = query.strip_prefix('@') {
            (SearchScope::Agents, rest.trim_start())
        } else if let Some(rest) = query.strip_prefix('#') {
            (SearchScope::Tasks, rest.trim_start())
        } else if let Some(rest) = query.strip_prefix("n:") {
            (SearchScope::Nodes, rest.trim_start())
        } else {
            (SearchScope::All, query)
        }
    }
}

// ---------------------------------------------------------------------------
// SearchItemKind / SearchItem
// ---------------------------------------------------------------------------

/// The kind of a search result item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchItemKind {
    /// A canvas node.
    Node,
    /// A running or registered agent.
    Agent,
    /// A task in the queue or history.
    Task,
    /// A canvas command.
    Command,
}

/// A single search result item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchItem {
    /// Unique item identifier.
    pub id: String,
    /// Item kind.
    pub kind: SearchItemKind,
    /// Primary display label.
    pub label: String,
    /// Optional subtitle (e.g. node type, agent role, task status).
    pub subtitle: Option<String>,
    /// Matched score in `[0.0, 1.0]`.
    pub score: f32,
    /// Optional associated dimension ID.
    pub dimension_id: Option<String>,
}

// ---------------------------------------------------------------------------
// SearchIndex
// ---------------------------------------------------------------------------

/// An in-memory index of searchable items.
///
/// Items are stored by ID; the index supports simple case-insensitive
/// substring matching.  A production implementation would use a fuzzy-match
/// algorithm (e.g., Smith-Waterman or fzf-style scoring).
#[derive(Debug, Default, Clone)]
pub struct SearchIndex {
    items: HashMap<String, SearchItem>,
}

impl SearchIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update an item in the index.
    pub fn upsert(&mut self, item: SearchItem) {
        self.items.insert(item.id.clone(), item);
    }

    /// Remove an item by ID.
    pub fn remove(&mut self, id: &str) {
        self.items.remove(id);
    }

    /// Number of indexed items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Search the index, returning results sorted by score descending.
    ///
    /// The `raw_query` string is parsed for prefix-based scope narrowing.
    /// Matching is case-insensitive substring search.
    pub fn search(&self, raw_query: &str) -> Vec<SearchItem> {
        let (scope, query) = SearchScope::from_prefix(raw_query);
        let query_lower = query.to_lowercase();

        let mut results: Vec<SearchItem> = self
            .items
            .values()
            .filter(|item| {
                // Scope filter.
                let kind_matches = match scope {
                    SearchScope::All => true,
                    SearchScope::Commands => item.kind == SearchItemKind::Command,
                    SearchScope::Agents => item.kind == SearchItemKind::Agent,
                    SearchScope::Tasks => item.kind == SearchItemKind::Task,
                    SearchScope::Nodes => item.kind == SearchItemKind::Node,
                };
                if !kind_matches {
                    return false;
                }
                // Text filter.
                query_lower.is_empty()
                    || item.label.to_lowercase().contains(&query_lower)
                    || item
                        .subtitle
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query_lower)
            })
            .map(|item| {
                // Simple scoring: exact label match scores highest.
                let score = if item.label.to_lowercase() == query_lower {
                    1.0_f32
                } else if item.label.to_lowercase().starts_with(&query_lower) {
                    0.8_f32
                } else {
                    0.5_f32
                };
                SearchItem {
                    score,
                    ..item.clone()
                }
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results
    }
}

// ---------------------------------------------------------------------------
// CommandRegistry
// ---------------------------------------------------------------------------

/// A registered canvas command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasCommand {
    /// Command identifier (kebab-case, e.g. `"fit-selection"`).
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Optional keyboard shortcut description.
    pub shortcut: Option<String>,
    /// Command category.
    pub category: String,
}

/// Registry of all available canvas commands.
#[derive(Debug, Default, Clone)]
pub struct CommandRegistry {
    commands: HashMap<String, CanvasCommand>,
}

impl CommandRegistry {
    /// Create a registry with the default set of canvas commands.
    pub fn default_canvas() -> Self {
        let mut r = Self::default();
        let defaults = vec![
            ("fit-selection", "Fit Selection", Some("F"), "Navigation"),
            ("zoom-in", "Zoom In", Some("+"), "Navigation"),
            ("zoom-out", "Zoom Out", Some("-"), "Navigation"),
            ("zoom-reset", "Reset Zoom", Some("0"), "Navigation"),
            ("toggle-minimap", "Toggle Minimap", Some("M"), "Navigation"),
            ("select-all", "Select All", Some("Ctrl+A"), "Selection"),
            ("open-inspector", "Open Inspector", Some("Enter"), "Inspect"),
            ("open-command-palette", "Command Palette", Some("Ctrl+F"), "General"),
            ("undo", "Undo", Some("Ctrl+Z"), "Edit"),
            ("redo", "Redo", Some("Ctrl+Y"), "Edit"),
            ("delete-selected", "Delete Selected", Some("Del"), "Edit"),
            ("add-node", "Add Node", None, "Graph"),
            ("focus-mode", "Focus Mode", None, "Navigation"),
        ];
        for (id, label, shortcut, category) in defaults {
            r.register(CanvasCommand {
                id: id.into(),
                label: label.into(),
                shortcut: shortcut.map(Into::into),
                category: category.into(),
            });
        }
        r
    }

    /// Register a command.
    pub fn register(&mut self, cmd: CanvasCommand) {
        self.commands.insert(cmd.id.clone(), cmd);
    }

    /// Look up a command by ID.
    pub fn get(&self, id: &str) -> Option<&CanvasCommand> {
        self.commands.get(id)
    }

    /// Return all commands sorted by label.
    pub fn all_sorted(&self) -> Vec<&CanvasCommand> {
        let mut v: Vec<&CanvasCommand> = self.commands.values().collect();
        v.sort_by_key(|c| c.label.as_str());
        v
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index() -> SearchIndex {
        let mut idx = SearchIndex::new();
        idx.upsert(SearchItem {
            id: "n1".into(),
            kind: SearchItemKind::Node,
            label: "HTTP Request".into(),
            subtitle: Some("http".into()),
            score: 0.0,
            dimension_id: None,
        });
        idx.upsert(SearchItem {
            id: "a1".into(),
            kind: SearchItemKind::Agent,
            label: "Trading Agent".into(),
            subtitle: Some("trading".into()),
            score: 0.0,
            dimension_id: None,
        });
        idx.upsert(SearchItem {
            id: "t1".into(),
            kind: SearchItemKind::Task,
            label: "Backtest Run".into(),
            subtitle: Some("queued".into()),
            score: 0.0,
            dimension_id: None,
        });
        idx
    }

    #[test]
    fn search_scope_prefix_parsing() {
        assert_eq!(SearchScope::from_prefix(">cmd").0, SearchScope::Commands);
        assert_eq!(SearchScope::from_prefix("@agent").0, SearchScope::Agents);
        assert_eq!(SearchScope::from_prefix("#task").0, SearchScope::Tasks);
        assert_eq!(SearchScope::from_prefix("n:node").0, SearchScope::Nodes);
        assert_eq!(SearchScope::from_prefix("free").0, SearchScope::All);
    }

    #[test]
    fn search_all_returns_multiple() {
        let idx = make_index();
        let results = idx.search("");
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn search_scope_agents_only() {
        let idx = make_index();
        let results = idx.search("@");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, SearchItemKind::Agent);
    }

    #[test]
    fn search_substring_filter() {
        let idx = make_index();
        let results = idx.search("http");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "n1");
    }

    #[test]
    fn command_registry_default_commands() {
        let reg = CommandRegistry::default_canvas();
        assert!(reg.get("fit-selection").is_some());
        assert!(reg.get("undo").is_some());
        assert!(reg.get("add-node").is_some());
    }

    #[test]
    fn command_registry_sorted() {
        let reg = CommandRegistry::default_canvas();
        let sorted = reg.all_sorted();
        let labels: Vec<&str> = sorted.iter().map(|c| c.label.as_str()).collect();
        let mut expected = labels.clone();
        expected.sort();
        assert_eq!(labels, expected);
    }
}
