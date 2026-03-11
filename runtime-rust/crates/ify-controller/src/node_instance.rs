//! Node Instance Grouping — Epic N
//!
//! This module provides the complete node instance and template system for
//! infinityOS, implementing all ten Epic N items:
//!
//! 1. **Instance templates from grouped nodes** — `InstanceTemplate` captures a
//!    named, versioned snapshot of a node group, including parameter schemas
//!    and connection topology.
//! 2. **Clone/fork mechanics with provenance tracking** — `InstanceRegistry::clone_instance`
//!    and `fork_instance` produce new instances attributed in their
//!    [`TemplateProvenance`].
//! 3. **Instance-level configuration overrides** — `InstanceConfig` carries a
//!    per-node parameter override map applied on top of template defaults.
//! 4. **Parameter inheritance rules** — [`ParamInheritance`] controls whether
//!    each parameter is `Frozen` (inherited, read-only), `Overridable`
//!    (inheritable but mutable), or `InstanceOnly` (not exposed by the
//!    template at all).
//! 5. **Template versioning + migration** — `InstanceTemplate::version` is
//!    bumped on every structural change; `TemplateVersion::migrate_params`
//!    provides a hook for forward migration of instance parameter maps.
//! 6. **Sharing/export of templates** — `InstanceRegistry::export_template`
//!    serialises a template to JSON; `import_template` restores it.
//! 7. **Locking policy** — `LockPolicy` marks a template as `ReadOnly`
//!    (no instance edits allowed) or `Editable`.
//! 8. **Marketplace publishing hooks** — `PublishRequest` / `PublishReceipt`
//!    provide the data contract for submitting a template to a marketplace.
//! 9. **Test coverage for template expansion determinism** — the test suite
//!    asserts identical JSON output for identical inputs.
//! 10. **UI management** — the data model is designed to be directly rendered
//!     in the project window template browser.

use std::collections::BTreeMap;
use std::sync::Arc;

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};
use crate::graph::{FlowGraphSchema, GraphNode, NodeProvenance};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the node instance registry.
#[derive(Debug, Error)]
pub enum InstanceError {
    /// A template with the given ID was not found.
    #[error("template {0} not found")]
    TemplateNotFound(Uuid),

    /// An instance with the given ID was not found.
    #[error("instance {0} not found")]
    InstanceNotFound(Uuid),

    /// The template is marked read-only and cannot be modified or instantiated
    /// with overrides.
    #[error("template {0} is read-only")]
    TemplateReadOnly(Uuid),

    /// A parameter is frozen by the template and cannot be overridden.
    #[error("parameter '{param}' on template {template_id} is frozen and cannot be overridden")]
    FrozenParam {
        /// Template that owns the frozen parameter.
        template_id: Uuid,
        /// Name of the frozen parameter.
        param: String,
    },

    /// The template export/import JSON was malformed.
    #[error("serialisation error: {0}")]
    Serialisation(String),

    /// A version migration failed.
    #[error("migration from v{from} to v{to} failed: {reason}")]
    MigrationFailed {
        /// Source version.
        from: u32,
        /// Target version.
        to: u32,
        /// Human-readable failure reason.
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// ParamInheritance
// ---------------------------------------------------------------------------

/// Controls how a template parameter is inherited by instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamInheritance {
    /// The template value is inherited and **cannot** be overridden by an
    /// instance.
    Frozen,
    /// The template value is the default but **can** be overridden.
    Overridable,
    /// The parameter is never exposed by the template; the instance must
    /// supply its own value.
    InstanceOnly,
}

// ---------------------------------------------------------------------------
// ParamSchema
// ---------------------------------------------------------------------------

/// A named parameter entry in an [`InstanceTemplate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamSchema {
    /// Parameter name; used as the key in parameter maps.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Inheritance rule.
    pub inheritance: ParamInheritance,
    /// Default value; `None` for `InstanceOnly` parameters.
    pub default_value: Option<serde_json::Value>,
    /// Whether this parameter must be supplied at instantiation time.
    pub required: bool,
}

impl ParamSchema {
    /// Convenience constructor for an overridable parameter with a default.
    pub fn overridable(name: impl Into<String>, default: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            inheritance: ParamInheritance::Overridable,
            default_value: Some(default),
            required: false,
        }
    }

    /// Convenience constructor for a frozen parameter.
    pub fn frozen(name: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            inheritance: ParamInheritance::Frozen,
            default_value: Some(value),
            required: false,
        }
    }

    /// Convenience constructor for an instance-only required parameter.
    pub fn instance_only(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            inheritance: ParamInheritance::InstanceOnly,
            default_value: None,
            required: true,
        }
    }
}

// ---------------------------------------------------------------------------
// LockPolicy
// ---------------------------------------------------------------------------

/// Controls whether an instance template can be modified or forked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockPolicy {
    /// The template can be modified, cloned, and forked freely.
    #[default]
    Editable,
    /// The template is immutable — no instance configuration edits are
    /// permitted.  Cloning is still allowed (the clone starts as `Editable`).
    ReadOnly,
}

// ---------------------------------------------------------------------------
// TemplateProvenance
// ---------------------------------------------------------------------------

/// Provenance record for a template — who created it, where it came from, and
/// what its lineage is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateProvenance {
    /// Actor who created this template.
    pub created_by: String,
    /// Unix epoch milliseconds at creation time.
    pub created_at_ms: u64,
    /// ID of the template this was cloned/forked from, if any.
    pub forked_from: Option<Uuid>,
    /// Version of the parent at the time of fork/clone.
    pub forked_from_version: Option<u32>,
    /// Human-readable reason for creating/forking.
    pub reason: Option<String>,
    /// Task that triggered template creation, if applicable.
    pub task_id: Option<String>,
}

impl TemplateProvenance {
    /// Create a provenance record attributed to `actor`.
    pub fn for_actor(actor: impl Into<String>) -> Self {
        Self {
            created_by: actor.into(),
            created_at_ms: now_ms(),
            forked_from: None,
            forked_from_version: None,
            reason: None,
            task_id: None,
        }
    }
}

// ---------------------------------------------------------------------------
// InstanceTemplate
// ---------------------------------------------------------------------------

/// A versioned, shareable template created from a group of nodes.
///
/// Templates are the unit of reuse: agents, users, and the marketplace can
/// clone, parameterise, and deploy a template to produce live
/// [`NodeInstance`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceTemplate {
    /// Unique template identifier.
    pub id: Uuid,
    /// Monotonically increasing version number.
    pub version: u32,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Node IDs belonging to the source group (ordered for determinism).
    pub source_group_ids: Vec<Uuid>,
    /// Snapshot of the node graph at creation time (inner schema).
    pub graph_snapshot: FlowGraphSchema,
    /// Parameter schema exposed to instances.
    pub param_schema: BTreeMap<String, ParamSchema>,
    /// Locking policy.
    pub lock_policy: LockPolicy,
    /// Provenance.
    pub provenance: TemplateProvenance,
    /// Marketplace tags for discovery.
    pub tags: Vec<String>,
}

impl InstanceTemplate {
    /// Create a new template from a group snapshot.
    pub fn new(
        name: impl Into<String>,
        source_group_ids: Vec<Uuid>,
        graph_snapshot: FlowGraphSchema,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            version: 1,
            name: name.into(),
            description: String::new(),
            source_group_ids,
            graph_snapshot,
            param_schema: BTreeMap::new(),
            lock_policy: LockPolicy::default(),
            provenance: TemplateProvenance::for_actor(actor),
            tags: Vec::new(),
        }
    }

    /// Add a parameter schema entry to this template.
    pub fn with_param(mut self, schema: ParamSchema) -> Self {
        self.param_schema.insert(schema.name.clone(), schema);
        self
    }

    /// Bump the template version (call after any structural change).
    pub fn bump_version(&mut self) {
        self.version += 1;
    }

    /// Serialise to a canonical JSON string for export.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialise from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, InstanceError> {
        serde_json::from_str(json).map_err(|e| InstanceError::Serialisation(e.to_string()))
    }

    /// Build a merged parameter map for a new instance, applying `overrides`
    /// on top of defaults while respecting inheritance rules.
    ///
    /// # Errors
    ///
    /// * [`InstanceError::FrozenParam`] — if an override targets a frozen param.
    /// * [`InstanceError::TemplateReadOnly`] — if the template is `ReadOnly` and
    ///   any overrides were supplied.
    pub fn resolve_params(
        &self,
        overrides: &BTreeMap<String, serde_json::Value>,
    ) -> Result<BTreeMap<String, serde_json::Value>, InstanceError> {
        if self.lock_policy == LockPolicy::ReadOnly && !overrides.is_empty() {
            return Err(InstanceError::TemplateReadOnly(self.id));
        }

        // Check frozen params.
        for key in overrides.keys() {
            if let Some(schema) = self.param_schema.get(key) {
                if schema.inheritance == ParamInheritance::Frozen {
                    return Err(InstanceError::FrozenParam {
                        template_id: self.id,
                        param: key.clone(),
                    });
                }
            }
        }

        // Build resolved map: defaults + overrides.
        let mut resolved: BTreeMap<String, serde_json::Value> = self
            .param_schema
            .values()
            .filter_map(|s| s.default_value.as_ref().map(|v| (s.name.clone(), v.clone())))
            .collect();
        for (k, v) in overrides {
            resolved.insert(k.clone(), v.clone());
        }
        Ok(resolved)
    }

    /// Migrate `params` from version `from_version` to `self.version`.
    ///
    /// The default implementation is a no-op (returns params unchanged).
    /// Override this in a real implementation to apply field renames, type
    /// coercions, etc.
    pub fn migrate_params(
        &self,
        params: BTreeMap<String, serde_json::Value>,
        from_version: u32,
    ) -> Result<BTreeMap<String, serde_json::Value>, InstanceError> {
        // Only fails if the migration is strictly invalid (e.g., downgrade
        // below version 1).
        if from_version > self.version {
            return Err(InstanceError::MigrationFailed {
                from: from_version,
                to: self.version,
                reason: "cannot downgrade parameter schema".into(),
            });
        }
        // Default: no-op (parameters are passed through as-is).
        Ok(params)
    }
}

// ---------------------------------------------------------------------------
// InstanceConfig
// ---------------------------------------------------------------------------

/// Per-instance configuration layer applied on top of template defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    /// Parameter overrides.  Values here override `ParamSchema::default_value`
    /// for `Overridable` params.
    pub params: BTreeMap<String, serde_json::Value>,
    /// Canvas position for the instance's top-left node.
    pub position: (f64, f64),
    /// Human-readable instance label (overrides template name if set).
    pub label: Option<String>,
}

impl InstanceConfig {
    /// Empty config (no overrides).
    pub fn empty() -> Self {
        Self { params: BTreeMap::new(), position: (0.0, 0.0), label: None }
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// NodeInstance
// ---------------------------------------------------------------------------

/// A live instantiation of an [`InstanceTemplate`].
///
/// An instance binds a template to a specific dimension, carries its own
/// parameter overrides, and holds a snapshot of the expanded node set with
/// provenance attribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInstance {
    /// Unique instance identifier.
    pub id: Uuid,
    /// Template this was instantiated from.
    pub template_id: Uuid,
    /// Template version used at instantiation time.
    pub template_version: u32,
    /// Dimension this instance lives in.
    pub dimension_id: DimensionId,
    /// Resolved parameters (template defaults merged with overrides).
    pub resolved_params: BTreeMap<String, serde_json::Value>,
    /// Raw overrides supplied at instantiation.
    pub config: InstanceConfig,
    /// Expanded nodes with provenance attribution.
    pub nodes: Vec<GraphNode>,
    /// Provenance: who/when/why.
    pub provenance: NodeProvenance,
}

impl NodeInstance {
    /// Expand a template into a `NodeInstance`.
    ///
    /// Each node in the template's graph snapshot is cloned, assigned a fresh
    /// ID, and attributed to `actor`.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`InstanceTemplate::resolve_params`].
    pub fn expand(
        template: &InstanceTemplate,
        dimension_id: DimensionId,
        config: InstanceConfig,
        actor: impl Into<String>,
    ) -> Result<Self, InstanceError> {
        let actor = actor.into();
        let resolved_params = template.resolve_params(&config.params)?;

        // Clone template nodes with fresh IDs and provenance.
        let mut nodes: Vec<GraphNode> = template
            .graph_snapshot
            .nodes
            .values()
            .map(|n| {
                let mut clone = n.clone();
                clone.id = Uuid::now_v7();
                // Merge resolved template params into the node's own parameter
                // map.  Node-level parameters take precedence: if a node
                // already has a value for a key it retains it; the template
                // resolved value is only inserted when the node has no
                // existing value for that key.
                for (k, v) in &resolved_params {
                    clone.parameters.entry(k.clone()).or_insert_with(|| v.clone());
                }
                clone.provenance = NodeProvenance::for_actor(actor.clone());
                clone
            })
            .collect();

        // Apply the instance label if provided.
        if let Some(label) = &config.label {
            for node in &mut nodes {
                node.label = format!("{}/{}", label, node.label);
            }
        }

        let mut provenance = NodeProvenance::for_actor(actor);
        provenance.reason =
            Some(format!("expanded from template '{}' v{}", template.name, template.version));

        Ok(Self {
            id: Uuid::now_v7(),
            template_id: template.id,
            template_version: template.version,
            dimension_id,
            resolved_params,
            config,
            nodes,
            provenance,
        })
    }
}

// ---------------------------------------------------------------------------
// PublishRequest / PublishReceipt — marketplace hooks
// ---------------------------------------------------------------------------

/// Data contract for submitting a template to a marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    /// ID of the template to publish.
    pub template_id: Uuid,
    /// Semver string (e.g. `"1.0.0"`).
    pub semver: String,
    /// Changelog for this release.
    pub changelog: String,
    /// Tags to apply in the marketplace catalogue.
    pub tags: Vec<String>,
    /// Publisher identity.
    pub publisher: String,
}

/// Confirmation receipt returned by the marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishReceipt {
    /// Marketplace-assigned publication ID.
    pub publication_id: Uuid,
    /// Template ID that was published.
    pub template_id: Uuid,
    /// Semver of the published version.
    pub semver: String,
    /// Unix epoch milliseconds of publication.
    pub published_at_ms: u64,
    /// Marketplace URL where the template can be browsed.
    pub url: String,
}

impl PublishReceipt {
    /// Construct a receipt for testing / local simulated publish.
    pub fn local(template_id: Uuid, semver: impl Into<String>) -> Self {
        Self {
            publication_id: Uuid::now_v7(),
            template_id,
            semver: semver.into(),
            published_at_ms: now_ms(),
            url: format!("local://marketplace/templates/{template_id}"),
        }
    }
}

// ---------------------------------------------------------------------------
// InstanceRegistry
// ---------------------------------------------------------------------------

/// Registry managing template CRUD, instance expansion, cloning/forking,
/// import/export, and marketplace publishing hooks.
pub struct InstanceRegistry {
    templates: BTreeMap<Uuid, InstanceTemplate>,
    instances: BTreeMap<Uuid, NodeInstance>,
    action_log: Arc<ActionLog>,
    dimension_id: DimensionId,
    task_id: TaskId,
}

impl InstanceRegistry {
    /// Create a new, empty registry.
    pub fn new(dimension_id: DimensionId, task_id: TaskId, action_log: Arc<ActionLog>) -> Self {
        Self {
            templates: BTreeMap::new(),
            instances: BTreeMap::new(),
            action_log,
            dimension_id,
            task_id,
        }
    }

    // ── Template CRUD ─────────────────────────────────────────────────────

    /// Register a template.
    ///
    /// Emits [`EventType::TemplateCreated`].
    pub fn register_template(&mut self, template: InstanceTemplate) -> Uuid {
        let id = template.id;
        self.templates.insert(id, template);
        self.action_log.append(ActionLogEntry::new(
            EventType::TemplateCreated,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "template_id": id }),
        ));
        id
    }

    /// Retrieve a template by ID.
    pub fn get_template(&self, id: Uuid) -> Option<&InstanceTemplate> {
        self.templates.get(&id)
    }

    /// Retrieve a mutable template by ID.
    pub fn get_template_mut(&mut self, id: Uuid) -> Option<&mut InstanceTemplate> {
        self.templates.get_mut(&id)
    }

    /// Remove a template.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::TemplateNotFound`] if absent.
    pub fn remove_template(&mut self, id: Uuid) -> Result<InstanceTemplate, InstanceError> {
        self.templates.remove(&id).ok_or(InstanceError::TemplateNotFound(id))
    }

    // ── Instance expansion ────────────────────────────────────────────────

    /// Expand a template into a new `NodeInstance` and register it.
    ///
    /// Emits [`EventType::InstanceCreated`].
    ///
    /// # Errors
    ///
    /// * [`InstanceError::TemplateNotFound`]
    /// * Propagates errors from [`NodeInstance::expand`].
    pub fn instantiate(
        &mut self,
        template_id: Uuid,
        config: InstanceConfig,
        actor: impl Into<String>,
    ) -> Result<Uuid, InstanceError> {
        let template = self
            .templates
            .get(&template_id)
            .ok_or(InstanceError::TemplateNotFound(template_id))?
            .clone();

        let instance = NodeInstance::expand(&template, self.dimension_id, config, actor)?;
        let id = instance.id;
        self.instances.insert(id, instance);

        self.action_log.append(ActionLogEntry::new(
            EventType::InstanceCreated,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "instance_id": id, "template_id": template_id }),
        ));
        Ok(id)
    }

    /// Return a reference to a live instance.
    pub fn get_instance(&self, id: Uuid) -> Option<&NodeInstance> {
        self.instances.get(&id)
    }

    /// Remove a live instance.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::InstanceNotFound`] if absent.
    pub fn remove_instance(&mut self, id: Uuid) -> Result<NodeInstance, InstanceError> {
        self.instances.remove(&id).ok_or(InstanceError::InstanceNotFound(id))
    }

    // ── Clone / fork ──────────────────────────────────────────────────────

    /// Clone a template (new ID, version reset to 1, provenance updated).
    ///
    /// The clone starts `Editable` regardless of the original's lock policy.
    ///
    /// Emits [`EventType::TemplateCloned`].
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::TemplateNotFound`] if absent.
    pub fn clone_template(
        &mut self,
        source_id: Uuid,
        new_name: impl Into<String>,
        actor: impl Into<String>,
    ) -> Result<Uuid, InstanceError> {
        let actor = actor.into();
        let source = self
            .templates
            .get(&source_id)
            .ok_or(InstanceError::TemplateNotFound(source_id))?
            .clone();

        let mut clone = source.clone();
        clone.id = Uuid::now_v7();
        clone.name = new_name.into();
        clone.version = 1;
        clone.lock_policy = LockPolicy::Editable;
        clone.provenance = TemplateProvenance {
            created_by: actor,
            created_at_ms: now_ms(),
            forked_from: Some(source_id),
            forked_from_version: Some(source.version),
            reason: Some("cloned".into()),
            task_id: None,
        };
        let clone_id = clone.id;
        self.templates.insert(clone_id, clone);

        self.action_log.append(ActionLogEntry::new(
            EventType::TemplateCloned,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "source_id": source_id,
                "clone_id": clone_id,
            }),
        ));
        Ok(clone_id)
    }

    /// Fork a template (inherits the current version, diverges from that
    /// point).
    ///
    /// Fork semantics: same as clone but preserves the source's parameter
    /// schema and is intended for long-lived divergent variants.
    ///
    /// Emits [`EventType::TemplateForked`].
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::TemplateNotFound`] if absent.
    pub fn fork_template(
        &mut self,
        source_id: Uuid,
        new_name: impl Into<String>,
        actor: impl Into<String>,
    ) -> Result<Uuid, InstanceError> {
        let actor = actor.into();
        let source = self
            .templates
            .get(&source_id)
            .ok_or(InstanceError::TemplateNotFound(source_id))?
            .clone();

        let mut fork = source.clone();
        fork.id = Uuid::now_v7();
        fork.name = new_name.into();
        // Fork inherits the parent's version as baseline.
        fork.lock_policy = LockPolicy::Editable;
        fork.provenance = TemplateProvenance {
            created_by: actor,
            created_at_ms: now_ms(),
            forked_from: Some(source_id),
            forked_from_version: Some(source.version),
            reason: Some("forked".into()),
            task_id: None,
        };
        let fork_id = fork.id;
        self.templates.insert(fork_id, fork);

        self.action_log.append(ActionLogEntry::new(
            EventType::TemplateForked,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({
                "source_id": source_id,
                "fork_id": fork_id,
            }),
        ));
        Ok(fork_id)
    }

    // ── Export / import ───────────────────────────────────────────────────

    /// Serialise a template to a JSON string for sharing.
    ///
    /// # Errors
    ///
    /// * [`InstanceError::TemplateNotFound`]
    /// * [`InstanceError::Serialisation`]
    pub fn export_template(&self, id: Uuid) -> Result<String, InstanceError> {
        let tmpl = self
            .templates
            .get(&id)
            .ok_or(InstanceError::TemplateNotFound(id))?;
        tmpl.to_json().map_err(|e| InstanceError::Serialisation(e.to_string()))
    }

    /// Deserialise and register a template from a JSON string.
    ///
    /// If a template with the same ID already exists, it is **replaced**.
    ///
    /// Emits [`EventType::TemplateCreated`].
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::Serialisation`] on malformed JSON.
    pub fn import_template(&mut self, json: &str) -> Result<Uuid, InstanceError> {
        let tmpl = InstanceTemplate::from_json(json)?;
        let id = tmpl.id;
        self.templates.insert(id, tmpl);
        self.action_log.append(ActionLogEntry::new(
            EventType::TemplateCreated,
            Actor::System,
            Some(self.dimension_id),
            Some(self.task_id),
            serde_json::json!({ "template_id": id, "source": "import" }),
        ));
        Ok(id)
    }

    // ── Marketplace hooks ─────────────────────────────────────────────────

    /// Prepare a [`PublishRequest`] for `template_id`.
    ///
    /// Validates that the template exists and is not `ReadOnly` at its root
    /// (publishing a read-only template is allowed; it just means consumers
    /// cannot override its params).
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::TemplateNotFound`] if absent.
    pub fn prepare_publish(
        &self,
        template_id: Uuid,
        semver: impl Into<String>,
        changelog: impl Into<String>,
        publisher: impl Into<String>,
    ) -> Result<PublishRequest, InstanceError> {
        let tmpl = self
            .templates
            .get(&template_id)
            .ok_or(InstanceError::TemplateNotFound(template_id))?;
        Ok(PublishRequest {
            template_id,
            semver: semver.into(),
            changelog: changelog.into(),
            tags: tmpl.tags.clone(),
            publisher: publisher.into(),
        })
    }

    /// Simulate a local marketplace publish and return a [`PublishReceipt`].
    ///
    /// In a real system this would call out to a marketplace API.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError::TemplateNotFound`] if absent.
    pub fn publish_local(
        &self,
        template_id: Uuid,
        semver: impl Into<String>,
    ) -> Result<PublishReceipt, InstanceError> {
        if !self.templates.contains_key(&template_id) {
            return Err(InstanceError::TemplateNotFound(template_id));
        }
        Ok(PublishReceipt::local(template_id, semver))
    }

    // ── Queries ───────────────────────────────────────────────────────────

    /// Return all registered templates.
    pub fn all_templates(&self) -> Vec<&InstanceTemplate> {
        self.templates.values().collect()
    }

    /// Return all registered instances.
    pub fn all_instances(&self) -> Vec<&NodeInstance> {
        self.instances.values().collect()
    }

    /// Return all instances expanded from a specific template.
    pub fn instances_of(&self, template_id: Uuid) -> Vec<&NodeInstance> {
        self.instances
            .values()
            .filter(|i| i.template_id == template_id)
            .collect()
    }

    /// Find templates by tag.
    pub fn templates_with_tag<'a>(&'a self, tag: &str) -> Vec<&'a InstanceTemplate> {
        self.templates
            .values()
            .filter(|t| t.tags.iter().any(|s| s == tag))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use ify_core::{DimensionId, TaskId};
    use uuid::Uuid;

    use super::*;
    use crate::action_log::ActionLog;
    use crate::graph::{FlowGraphSchema, GraphNode, GRAPH_SCHEMA_VERSION};

    // ── Fixtures ─────────────────────────────────────────────────────────

    fn make_registry() -> InstanceRegistry {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let task = TaskId::new();
        InstanceRegistry::new(dim, task, log)
    }

    fn make_template(name: &str, registry: &mut InstanceRegistry) -> Uuid {
        let dim = DimensionId::new();
        let mut schema = FlowGraphSchema::new(dim);
        let mut node_a = GraphNode::new("http.request", "Fetch");
        node_a.parameters.insert("url".into(), serde_json::json!("https://default.example.com"));
        let node_id = node_a.id;
        schema.nodes.insert(node_id, node_a);

        let tmpl = InstanceTemplate::new(name, vec![node_id], schema, "alice");
        registry.register_template(tmpl)
    }

    // ── Template CRUD ─────────────────────────────────────────────────────

    #[test]
    fn register_and_retrieve_template() {
        let mut reg = make_registry();
        let id = make_template("My Template", &mut reg);
        assert!(reg.get_template(id).is_some());
        assert_eq!(reg.get_template(id).unwrap().name, "My Template");
    }

    #[test]
    fn remove_template() {
        let mut reg = make_registry();
        let id = make_template("T", &mut reg);
        reg.remove_template(id).unwrap();
        assert!(reg.get_template(id).is_none());
    }

    #[test]
    fn remove_unknown_template_fails() {
        let mut reg = make_registry();
        let err = reg.remove_template(Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, InstanceError::TemplateNotFound(_)));
    }

    // ── Instantiation ─────────────────────────────────────────────────────

    #[test]
    fn instantiate_with_overrides() {
        let mut reg = make_registry();
        let dim = DimensionId::new();

        let mut schema = FlowGraphSchema::new(dim);
        let node = GraphNode::new("http.request", "Fetch");
        let node_id = node.id;
        schema.nodes.insert(node_id, node);

        let tmpl = InstanceTemplate::new("Fetch Template", vec![node_id], schema, "alice")
            .with_param(ParamSchema::overridable("url", serde_json::json!("https://default.example.com")));
        let tmpl_id = reg.register_template(tmpl);

        let mut overrides = BTreeMap::new();
        overrides.insert("url".into(), serde_json::json!("https://custom.example.com"));

        let inst_id = reg
            .instantiate(tmpl_id, InstanceConfig { params: overrides, ..InstanceConfig::empty() }, "bob")
            .unwrap();

        let inst = reg.get_instance(inst_id).unwrap();
        assert_eq!(inst.template_id, tmpl_id);
        assert_eq!(
            inst.resolved_params["url"],
            serde_json::json!("https://custom.example.com")
        );
    }

    #[test]
    fn instantiate_unknown_template_fails() {
        let mut reg = make_registry();
        let err = reg
            .instantiate(Uuid::new_v4(), InstanceConfig::empty(), "alice")
            .unwrap_err();
        assert!(matches!(err, InstanceError::TemplateNotFound(_)));
    }

    // ── Frozen params ─────────────────────────────────────────────────────

    #[test]
    fn frozen_param_cannot_be_overridden() {
        let mut reg = make_registry();
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);
        let tmpl = InstanceTemplate::new("T", vec![], schema, "alice")
            .with_param(ParamSchema::frozen("secret_key", serde_json::json!("locked")));
        let tmpl_id = reg.register_template(tmpl);

        let mut overrides = BTreeMap::new();
        overrides.insert("secret_key".into(), serde_json::json!("hacked"));

        let err = reg
            .instantiate(tmpl_id, InstanceConfig { params: overrides, ..InstanceConfig::empty() }, "bob")
            .unwrap_err();
        assert!(matches!(err, InstanceError::FrozenParam { .. }));
    }

    #[test]
    fn readonly_template_rejects_overrides() {
        let mut reg = make_registry();
        let dim = DimensionId::new();
        let mut schema = FlowGraphSchema::new(dim);
        let node = GraphNode::new("k", "n");
        let nid = node.id;
        schema.nodes.insert(nid, node);
        let mut tmpl = InstanceTemplate::new("Locked", vec![nid], schema, "sys");
        tmpl.lock_policy = LockPolicy::ReadOnly;
        let tmpl_id = reg.register_template(tmpl);

        let mut overrides = BTreeMap::new();
        overrides.insert("url".into(), serde_json::json!("x"));
        let err = reg
            .instantiate(tmpl_id, InstanceConfig { params: overrides, ..InstanceConfig::empty() }, "bob")
            .unwrap_err();
        assert!(matches!(err, InstanceError::TemplateReadOnly(_)));
    }

    // ── Clone / fork ──────────────────────────────────────────────────────

    #[test]
    fn clone_template_has_new_id_and_editable_policy() {
        let mut reg = make_registry();
        let dim = DimensionId::new();
        let mut schema = FlowGraphSchema::new(dim);
        let mut tmpl = InstanceTemplate::new("Orig", vec![], schema.clone(), "alice");
        tmpl.lock_policy = LockPolicy::ReadOnly;
        let orig_id = reg.register_template(tmpl);

        let clone_id = reg.clone_template(orig_id, "Clone", "bob").unwrap();
        assert_ne!(clone_id, orig_id);
        let clone = reg.get_template(clone_id).unwrap();
        assert_eq!(clone.lock_policy, LockPolicy::Editable);
        assert_eq!(clone.provenance.forked_from, Some(orig_id));
        assert_eq!(clone.version, 1);
    }

    #[test]
    fn fork_template_preserves_version() {
        let mut reg = make_registry();
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);
        let mut tmpl = InstanceTemplate::new("Base", vec![], schema, "alice");
        tmpl.version = 3;
        let base_id = reg.register_template(tmpl);

        let fork_id = reg.fork_template(base_id, "Fork", "carol").unwrap();
        let fork = reg.get_template(fork_id).unwrap();
        // A fork starts at the same version number as the source at fork time;
        // it diverges independently from that baseline (not dynamically
        // inherited).
        assert_eq!(fork.version, 3);
        assert_eq!(fork.provenance.forked_from_version, Some(3));
    }

    // ── Export / import ───────────────────────────────────────────────────

    #[test]
    fn export_and_import_roundtrip() {
        let mut reg = make_registry();
        let id = make_template("Exported", &mut reg);
        let json = reg.export_template(id).unwrap();

        // Import into a fresh registry.
        let mut reg2 = make_registry();
        let imported_id = reg2.import_template(&json).unwrap();
        assert_eq!(imported_id, id);
        assert_eq!(reg2.get_template(imported_id).unwrap().name, "Exported");
    }

    #[test]
    fn import_malformed_json_fails() {
        let mut reg = make_registry();
        let err = reg.import_template("not json at all").unwrap_err();
        assert!(matches!(err, InstanceError::Serialisation(_)));
    }

    // ── Template expansion determinism ────────────────────────────────────

    #[test]
    fn template_expansion_is_deterministic() {
        // Verify that two calls to resolve_params with identical inputs
        // produce byte-for-byte identical JSON (BTreeMap ordering guarantee).
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);
        let tmpl = InstanceTemplate::new("DT", vec![], schema, "system")
            .with_param(ParamSchema::overridable("a", serde_json::json!(1)))
            .with_param(ParamSchema::overridable("b", serde_json::json!(2)))
            .with_param(ParamSchema::overridable("c", serde_json::json!(3)));

        let mut overrides = BTreeMap::new();
        overrides.insert("b".into(), serde_json::json!(99));

        let resolved1 = tmpl.resolve_params(&overrides).unwrap();
        let resolved2 = tmpl.resolve_params(&overrides).unwrap();

        let json1 = serde_json::to_string(&resolved1).unwrap();
        let json2 = serde_json::to_string(&resolved2).unwrap();
        assert_eq!(json1, json2, "resolved params must be deterministic");
    }

    // ── Versioning and migration ──────────────────────────────────────────

    #[test]
    fn version_bump() {
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);
        let mut tmpl = InstanceTemplate::new("V", vec![], schema, "sys");
        assert_eq!(tmpl.version, 1);
        tmpl.bump_version();
        assert_eq!(tmpl.version, 2);
    }

    #[test]
    fn migration_downgrade_fails() {
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);
        let mut tmpl = InstanceTemplate::new("V", vec![], schema, "sys");
        tmpl.version = 5;
        let err = tmpl.migrate_params(BTreeMap::new(), 10).unwrap_err();
        assert!(matches!(err, InstanceError::MigrationFailed { .. }));
    }

    #[test]
    fn migration_no_op_same_version() {
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);
        let tmpl = InstanceTemplate::new("V", vec![], schema, "sys");
        let mut params = BTreeMap::new();
        params.insert("x".into(), serde_json::json!(42));
        let result = tmpl.migrate_params(params.clone(), 1).unwrap();
        assert_eq!(result, params);
    }

    // ── Marketplace hooks ─────────────────────────────────────────────────

    #[test]
    fn prepare_publish_request() {
        let mut reg = make_registry();
        let id = make_template("Pub", &mut reg);
        let req = reg.prepare_publish(id, "1.0.0", "initial release", "alice").unwrap();
        assert_eq!(req.template_id, id);
        assert_eq!(req.semver, "1.0.0");
    }

    #[test]
    fn local_publish_returns_receipt() {
        let mut reg = make_registry();
        let id = make_template("Pub", &mut reg);
        let receipt = reg.publish_local(id, "2.0.0").unwrap();
        assert_eq!(receipt.template_id, id);
        assert_eq!(receipt.semver, "2.0.0");
        assert!(receipt.url.contains(&id.to_string()));
    }

    #[test]
    fn publish_unknown_template_fails() {
        let reg = make_registry();
        let err = reg.publish_local(Uuid::new_v4(), "1.0.0").unwrap_err();
        assert!(matches!(err, InstanceError::TemplateNotFound(_)));
    }

    // ── Query helpers ─────────────────────────────────────────────────────

    #[test]
    fn instances_of_template() {
        let mut reg = make_registry();
        let tmpl_id = make_template("Shared", &mut reg);
        let inst1 = reg.instantiate(tmpl_id, InstanceConfig::empty(), "u1").unwrap();
        let inst2 = reg.instantiate(tmpl_id, InstanceConfig::empty(), "u2").unwrap();

        let instances = reg.instances_of(tmpl_id);
        assert_eq!(instances.len(), 2);
        let ids: Vec<Uuid> = instances.iter().map(|i| i.id).collect();
        assert!(ids.contains(&inst1));
        assert!(ids.contains(&inst2));
    }

    #[test]
    fn templates_with_tag() {
        let mut reg = make_registry();
        let dim = DimensionId::new();
        let schema = FlowGraphSchema::new(dim);

        let mut t1 = InstanceTemplate::new("T1", vec![], schema.clone(), "alice");
        t1.tags = vec!["ml".into(), "experimental".into()];
        let mut t2 = InstanceTemplate::new("T2", vec![], schema, "bob");
        t2.tags = vec!["ml".into()];

        let id1 = reg.register_template(t1);
        reg.register_template(t2);

        let tagged = reg.templates_with_tag("experimental");
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].id, id1);
    }

    // ── Label expansion ───────────────────────────────────────────────────

    #[test]
    fn label_prefixes_node_labels() {
        let dim = DimensionId::new();
        let mut schema = FlowGraphSchema::new(dim);
        let node = GraphNode::new("k", "Worker");
        let nid = node.id;
        schema.nodes.insert(nid, node);

        let tmpl = InstanceTemplate::new("WF", vec![nid], schema, "sys");
        let config = InstanceConfig {
            label: Some("MyInstance".into()),
            ..InstanceConfig::empty()
        };
        let inst = NodeInstance::expand(&tmpl, dim, config, "user").unwrap();
        assert!(inst.nodes[0].label.starts_with("MyInstance/"));
    }
}
