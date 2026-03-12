//! Planner integration — converts agent plans into tasks and node-graph entries.
//!
//! The planner bridges high-level intent (a [`Plan`] with ordered [`PlanStep`]s)
//! and the low-level execution layer (a list of [`TaskId`]s and [`NodeRef`]s
//! ready for the orchestrator and node canvas).
//!
//! ## Flow
//!
//! ```text
//! User intent / LLM output
//!         │
//!         ▼
//!      Plan { steps: [...] }
//!         │
//!         ▼
//!  Planner::execute(&plan)
//!         │
//!         ▼
//!   PlannerResult { task_ids, node_refs }
//!         │
//!         ▼
//!  orchestrator.submit(task_id, ...)
//!  canvas.add_node(node_ref, ...)
//! ```
//!
//! ## Dependency ordering
//!
//! Plan steps may declare `depends_on` lists.  [`Planner::execute`] validates
//! that the dependency graph is acyclic before returning a result; a cyclic
//! dependency set returns [`PlannerError::CyclicDependency`].

use std::collections::{HashMap, VecDeque};

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the planner.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlannerError {
    /// A step references an unknown `step_id` in its `depends_on` list.
    #[error("unknown dependency: step {dependent} depends on unknown step {dependency}")]
    UnknownDependency {
        /// Step that declared the dependency.
        dependent: String,
        /// Step id that was not found.
        dependency: String,
    },

    /// The dependency graph contains a cycle.
    #[error("cyclic dependency detected involving step: {0}")]
    CyclicDependency(String),

    /// The plan contains no steps.
    #[error("plan has no steps")]
    EmptyPlan,

    /// Two steps share the same `step_id`.
    #[error("duplicate step id: {0}")]
    DuplicateStepId(String),
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

/// A reference to a canvas node generated for a plan step.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeRef {
    /// Step this node corresponds to.
    pub step_id: String,
    /// Stable node identifier (matches the TaskId for the step).
    pub node_id: TaskId,
}

/// A single step in an agent plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Unique identifier for this step within the plan.
    pub step_id: String,
    /// Human-readable description of what this step does.
    pub description: String,
    /// Optional tool to invoke (maps to a [`crate::tool_runner::ToolKind`] key).
    pub tool: Option<String>,
    /// JSON input payload forwarded to the tool or executor.
    pub input: serde_json::Value,
    /// Steps that must complete before this step may start.
    pub depends_on: Vec<String>,
}

impl PlanStep {
    /// Create a simple step with no dependencies and no tool.
    pub fn simple(step_id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            step_id: step_id.into(),
            description: description.into(),
            tool: None,
            input: serde_json::Value::Null,
            depends_on: vec![],
        }
    }
}

/// A complete agent plan ready for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Unique plan identifier.
    pub plan_id: TaskId,
    /// Dimension that owns this plan.
    pub dimension_id: DimensionId,
    /// Ordered list of plan steps (order is advisory; `depends_on` is authoritative).
    pub steps: Vec<PlanStep>,
}

impl Plan {
    /// Create a new plan for the given dimension.
    pub fn new(dimension_id: DimensionId, steps: Vec<PlanStep>) -> Self {
        Self {
            plan_id: TaskId::new(),
            dimension_id,
            steps,
        }
    }
}

// ---------------------------------------------------------------------------
// PlannerResult
// ---------------------------------------------------------------------------

/// The output of a successful planning pass.
#[derive(Debug, Clone)]
pub struct PlannerResult {
    /// One `TaskId` per plan step, in topological execution order.
    pub task_ids: Vec<TaskId>,
    /// Canvas node references, one per plan step.
    pub node_refs: Vec<NodeRef>,
    /// Mapping from `step_id` to assigned `TaskId` (for wiring up dependencies).
    pub step_task_map: HashMap<String, TaskId>,
}

// ---------------------------------------------------------------------------
// Planner
// ---------------------------------------------------------------------------

/// Converts agent [`Plan`]s into executable tasks and node references.
pub struct Planner;

impl Planner {
    /// Create a new planner instance (stateless).
    pub fn new() -> Self {
        Self
    }

    /// Execute the planner against the given plan.
    ///
    /// Returns a [`PlannerResult`] with task IDs and node refs in topological
    /// (dependency-respecting) order.
    ///
    /// # Errors
    ///
    /// - [`PlannerError::EmptyPlan`] — no steps provided.
    /// - [`PlannerError::DuplicateStepId`] — two steps share the same id.
    /// - [`PlannerError::UnknownDependency`] — a step depends on a non-existent step.
    /// - [`PlannerError::CyclicDependency`] — the dependency graph has a cycle.
    #[instrument(skip(self, plan), fields(plan_id = %plan.plan_id, steps = plan.steps.len()))]
    pub fn execute(&self, plan: &Plan) -> Result<PlannerResult, PlannerError> {
        if plan.steps.is_empty() {
            return Err(PlannerError::EmptyPlan);
        }

        // Build step-id → index map, checking for duplicates.
        let mut id_to_idx: HashMap<&str, usize> = HashMap::new();
        for (i, step) in plan.steps.iter().enumerate() {
            if id_to_idx.insert(step.step_id.as_str(), i).is_some() {
                return Err(PlannerError::DuplicateStepId(step.step_id.clone()));
            }
        }

        // Validate all dependency references.
        for step in &plan.steps {
            for dep in &step.depends_on {
                if !id_to_idx.contains_key(dep.as_str()) {
                    return Err(PlannerError::UnknownDependency {
                        dependent: step.step_id.clone(),
                        dependency: dep.clone(),
                    });
                }
            }
        }

        // Topological sort (Kahn's algorithm).
        let n = plan.steps.len();
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![vec![]; n];

        for (i, step) in plan.steps.iter().enumerate() {
            for dep in &step.depends_on {
                let j = *id_to_idx.get(dep.as_str()).unwrap();
                adj[j].push(i); // j must complete before i
                in_degree[i] += 1;
            }
        }

        let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut topo_order: Vec<usize> = Vec::with_capacity(n);

        while let Some(node) = queue.pop_front() {
            topo_order.push(node);
            for &next in &adj[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push_back(next);
                }
            }
        }

        if topo_order.len() != n {
            // Cycle detected — find a step still with in-degree > 0.
            let culprit = plan.steps.iter().enumerate().find_map(|(i, s)| {
                if in_degree[i] > 0 { Some(s.step_id.clone()) } else { None }
            }).unwrap_or_else(|| "unknown".to_owned());
            warn!(step = %culprit, "cyclic dependency detected");
            return Err(PlannerError::CyclicDependency(culprit));
        }

        // Assign TaskIds in topological order.
        let mut task_ids = Vec::with_capacity(n);
        let mut node_refs = Vec::with_capacity(n);
        let mut step_task_map = HashMap::with_capacity(n);

        for &idx in &topo_order {
            let step = &plan.steps[idx];
            let task_id = TaskId::new();
            debug!(step_id = %step.step_id, task_id = %task_id, "planner assigned task");
            task_ids.push(task_id);
            node_refs.push(NodeRef {
                step_id: step.step_id.clone(),
                node_id: task_id,
            });
            step_task_map.insert(step.step_id.clone(), task_id);
        }

        Ok(PlannerResult {
            task_ids,
            node_refs,
            step_task_map,
        })
    }
}

impl Default for Planner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::DimensionId;

    fn dim() -> DimensionId {
        DimensionId::new()
    }

    #[test]
    fn simple_plan_produces_task_per_step() {
        let plan = Plan::new(
            dim(),
            vec![
                PlanStep::simple("s1", "first"),
                PlanStep::simple("s2", "second"),
            ],
        );
        let result = Planner::new().execute(&plan).unwrap();
        assert_eq!(result.task_ids.len(), 2);
        assert_eq!(result.node_refs.len(), 2);
        assert_eq!(result.step_task_map.len(), 2);
    }

    #[test]
    fn dependency_ordering_respected() {
        // s1 → s2 → s3
        let plan = Plan::new(
            dim(),
            vec![
                PlanStep {
                    step_id: "s3".to_owned(),
                    description: "third".to_owned(),
                    tool: None,
                    input: serde_json::Value::Null,
                    depends_on: vec!["s2".to_owned()],
                },
                PlanStep {
                    step_id: "s2".to_owned(),
                    description: "second".to_owned(),
                    tool: None,
                    input: serde_json::Value::Null,
                    depends_on: vec!["s1".to_owned()],
                },
                PlanStep::simple("s1", "first"),
            ],
        );
        let result = Planner::new().execute(&plan).unwrap();
        let order: Vec<&str> = result.node_refs.iter().map(|n| n.step_id.as_str()).collect();
        // s1 must come before s2, s2 before s3
        let pos: HashMap<&str, usize> = order.iter().enumerate().map(|(i, s)| (*s, i)).collect();
        assert!(pos["s1"] < pos["s2"]);
        assert!(pos["s2"] < pos["s3"]);
    }

    #[test]
    fn empty_plan_returns_error() {
        let plan = Plan::new(dim(), vec![]);
        assert!(matches!(
            Planner::new().execute(&plan),
            Err(PlannerError::EmptyPlan)
        ));
    }

    #[test]
    fn duplicate_step_id_rejected() {
        let plan = Plan::new(
            dim(),
            vec![
                PlanStep::simple("s1", "first"),
                PlanStep::simple("s1", "duplicate"),
            ],
        );
        assert!(matches!(
            Planner::new().execute(&plan),
            Err(PlannerError::DuplicateStepId(_))
        ));
    }

    #[test]
    fn unknown_dependency_rejected() {
        let plan = Plan::new(
            dim(),
            vec![PlanStep {
                step_id: "s1".to_owned(),
                description: "step".to_owned(),
                tool: None,
                input: serde_json::Value::Null,
                depends_on: vec!["ghost".to_owned()],
            }],
        );
        assert!(matches!(
            Planner::new().execute(&plan),
            Err(PlannerError::UnknownDependency { .. })
        ));
    }

    #[test]
    fn cyclic_dependency_rejected() {
        // s1 depends on s2 and s2 depends on s1
        let plan = Plan::new(
            dim(),
            vec![
                PlanStep {
                    step_id: "s1".to_owned(),
                    description: "a".to_owned(),
                    tool: None,
                    input: serde_json::Value::Null,
                    depends_on: vec!["s2".to_owned()],
                },
                PlanStep {
                    step_id: "s2".to_owned(),
                    description: "b".to_owned(),
                    tool: None,
                    input: serde_json::Value::Null,
                    depends_on: vec!["s1".to_owned()],
                },
            ],
        );
        assert!(matches!(
            Planner::new().execute(&plan),
            Err(PlannerError::CyclicDependency(_))
        ));
    }

    #[test]
    fn step_task_map_has_all_steps() {
        let plan = Plan::new(
            dim(),
            vec![
                PlanStep::simple("alpha", "first"),
                PlanStep::simple("beta", "second"),
            ],
        );
        let result = Planner::new().execute(&plan).unwrap();
        assert!(result.step_task_map.contains_key("alpha"));
        assert!(result.step_task_map.contains_key("beta"));
    }
}
