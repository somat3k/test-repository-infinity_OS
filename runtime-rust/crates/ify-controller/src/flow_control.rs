//! Advanced flow control with ML-aware decisions for algorithmic environments.
//!
//! The flow control engine supports conditional branching, fallbacks, and
//! loop bounds.  Decisions can be driven by runtime metrics or ML model scores.
//! Each evaluation emits ActionLog events to maintain the audit trail.

use std::collections::HashMap;
use std::sync::Arc;

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors returned by flow control operations.
#[derive(Debug, Error)]
pub enum FlowControlError {
    /// A referenced step does not exist in the flow graph.
    #[error("flow step {0} not found")]
    StepNotFound(Uuid),

    /// A transition target was not found in the flow graph.
    #[error("transition target {target} missing from flow (from {from})")]
    TransitionTargetNotFound {
        /// Step emitting the transition.
        from: Uuid,
        /// Target step that was missing.
        target: Uuid,
    },

    /// A required metric was missing from the flow context.
    #[error("metric '{0}' missing from flow context")]
    MissingMetric(String),

    /// A model could not be evaluated.
    #[error("model '{model}' evaluation failed: {reason}")]
    ModelEvaluationFailed {
        /// Model identifier.
        model: String,
        /// Diagnostic reason.
        reason: String,
    },

    /// The step has exceeded its visit limit.
    #[error("flow step {step_id} exceeded visit limit {limit}")]
    MaxVisitsExceeded {
        /// Step that exceeded its limit.
        step_id: Uuid,
        /// Allowed number of visits.
        limit: u32,
    },

    /// A chat payload adapter failed to transform the request.
    #[error("chat adapter failed: {0}")]
    ChatAdapterFailed(String),

    /// An instruction references a source that does not exist.
    #[error("instruction {instruction_id} references missing source {source_id}")]
    InstructionSourceMissing {
        /// Instruction identifier.
        instruction_id: Uuid,
        /// Missing source identifier.
        source_id: Uuid,
    },

    /// No transition matched and no fallback was configured.
    #[error("flow step {0} has no matching transition or fallback")]
    NoMatchingTransition(Uuid),
}

// ---------------------------------------------------------------------------
// Flow context
// ---------------------------------------------------------------------------

/// Runtime data supplied when evaluating flow control decisions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowContext {
    /// Dimension scope for the flow.
    pub dimension_id: DimensionId,
    /// Task scope for the flow.
    pub task_id: TaskId,
    /// Numeric metrics available for flow decisions.
    pub metrics: HashMap<String, f64>,
    /// Feature payload used by ML models.
    pub features: serde_json::Value,
}

impl FlowContext {
    /// Create a new flow context with empty metrics and no features.
    pub fn new(dimension_id: DimensionId, task_id: TaskId) -> Self {
        Self {
            dimension_id,
            task_id,
            metrics: HashMap::new(),
            features: serde_json::Value::Null,
        }
    }
}

// ---------------------------------------------------------------------------
// Flow graph types
// ---------------------------------------------------------------------------

/// Terminal outcome for a flow graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowTerminalStatus {
    /// Flow completed successfully.
    Completed,
    /// Flow ended with an error.
    Failed,
    /// Flow was cancelled.
    Cancelled,
}

/// The kind of flow step being executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowStepKind {
    /// A concrete node execution step.
    Task {
        /// The node identifier to execute.
        node_id: Uuid,
    },
    /// A decision step that evaluates transitions.
    Decision {
        /// Human-readable summary of the decision logic.
        description: String,
    },
    /// Terminal step indicating a completed flow.
    Terminal {
        /// Final status reported by the flow.
        status: FlowTerminalStatus,
    },
}

/// A transition between flow steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowTransition {
    /// Label for UI/debugging.
    pub label: String,
    /// Condition that must be satisfied to follow the transition.
    pub condition: FlowCondition,
    /// Target step identifier.
    pub target: Uuid,
}

impl FlowTransition {
    /// Create a new flow transition.
    pub fn new(label: impl Into<String>, condition: FlowCondition, target: Uuid) -> Self {
        Self {
            label: label.into(),
            condition,
            target,
        }
    }
}

/// Conditional logic used to select a transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowCondition {
    /// Always select this transition.
    Always,
    /// Select when the metric is greater than or equal to the threshold.
    MetricAtLeast {
        /// Metric name.
        metric: String,
        /// Threshold value.
        threshold: f64,
    },
    /// Select when the metric is less than the threshold.
    MetricBelow {
        /// Metric name.
        metric: String,
        /// Threshold value.
        threshold: f64,
    },
    /// Select when the ML model score is greater than or equal to the threshold.
    ModelScoreAtLeast {
        /// Model identifier.
        model: String,
        /// Score threshold.
        threshold: f64,
    },
}

/// A single step within a flow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    /// Unique step identifier.
    pub id: Uuid,
    /// Human-readable step name.
    pub name: String,
    /// Step behavior kind.
    pub kind: FlowStepKind,
    /// Ordered transitions to evaluate.
    pub transitions: Vec<FlowTransition>,
    /// Fallback step when no transition matches.
    pub fallback: Option<Uuid>,
    /// Optional maximum number of visits to this step.
    pub max_visits: Option<u32>,
}

impl FlowStep {
    /// Create a new flow step.
    pub fn new(name: impl Into<String>, kind: FlowStepKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            transitions: Vec::new(),
            fallback: None,
            max_visits: None,
        }
    }

    /// Create a task step targeting a node.
    pub fn task(name: impl Into<String>, node_id: Uuid) -> Self {
        Self::new(name, FlowStepKind::Task { node_id })
    }

    /// Create a decision step with a human-readable description.
    pub fn decision(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self::new(
            name,
            FlowStepKind::Decision {
                description: description.into(),
            },
        )
    }

    /// Create a terminal step with the specified status.
    pub fn terminal(name: impl Into<String>, status: FlowTerminalStatus) -> Self {
        Self::new(name, FlowStepKind::Terminal { status })
    }

    /// Append a transition to the step.
    pub fn add_transition(&mut self, transition: FlowTransition) {
        self.transitions.push(transition);
    }

    /// Set a fallback transition when no conditions match.
    pub fn with_fallback(mut self, fallback: Uuid) -> Self {
        self.fallback = Some(fallback);
        self
    }

    /// Set a maximum number of visits for the step.
    pub fn with_max_visits(mut self, max_visits: u32) -> Self {
        self.max_visits = Some(max_visits);
        self
    }
}

/// A full flow graph definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowGraph {
    /// Unique flow identifier.
    pub id: Uuid,
    /// Human-readable flow name.
    pub name: String,
    /// Entry step identifier.
    pub entrypoint: Uuid,
    /// Map of steps keyed by ID.
    pub steps: HashMap<Uuid, FlowStep>,
}

impl FlowGraph {
    /// Create a new flow graph with the given entry step.
    pub fn new(name: impl Into<String>, entry: FlowStep) -> Self {
        let entry_id = entry.id;
        let mut steps = HashMap::new();
        steps.insert(entry_id, entry);
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            entrypoint: entry_id,
            steps,
        }
    }

    /// Insert an additional step into the graph.
    pub fn insert_step(&mut self, step: FlowStep) {
        self.steps.insert(step.id, step);
    }

    /// Retrieve a step by ID.
    pub fn step(&self, id: Uuid) -> Option<&FlowStep> {
        self.steps.get(&id)
    }

    /// Validate that all transitions and fallbacks reference existing steps.
    pub fn validate(&self) -> Result<(), FlowControlError> {
        for (id, step) in &self.steps {
            for transition in &step.transitions {
                if !self.steps.contains_key(&transition.target) {
                    return Err(FlowControlError::TransitionTargetNotFound {
                        from: *id,
                        target: transition.target,
                    });
                }
            }
            if let Some(fallback) = step.fallback {
                if !self.steps.contains_key(&fallback) {
                    return Err(FlowControlError::TransitionTargetNotFound {
                        from: *id,
                        target: fallback,
                    });
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Agentic chat payload types
// ---------------------------------------------------------------------------

/// Source metadata for instructions used in chat payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionSource {
    /// Unique source identifier.
    pub id: Uuid,
    /// Source kind (dataset, spec, policy, etc.).
    pub kind: String,
    /// Optional URI where the source lives.
    pub uri: Option<String>,
    /// Short description of the source.
    pub summary: String,
}

impl InstructionSource {
    /// Create a new instruction source.
    pub fn new(kind: impl Into<String>, uri: Option<String>, summary: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: kind.into(),
            uri,
            summary: summary.into(),
        }
    }
}

/// Instruction entry within a dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionEntry {
    /// Unique instruction identifier.
    pub id: Uuid,
    /// Instruction content.
    pub content: String,
    /// Tags used to filter instructions.
    pub tags: Vec<String>,
    /// Source identifier for the instruction.
    pub source_id: Uuid,
}

impl InstructionEntry {
    /// Create a new instruction entry.
    pub fn new(content: impl Into<String>, tags: Vec<String>, source_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            content: content.into(),
            tags,
            source_id,
        }
    }
}

/// Instruction dataset used to assemble chat payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionDataset {
    /// Dataset identifier.
    pub id: Uuid,
    /// Dataset name.
    pub name: String,
    /// Dataset version identifier.
    pub version: String,
    /// Instruction entries.
    pub instructions: Vec<InstructionEntry>,
    /// Sources referenced by instructions.
    pub sources: Vec<InstructionSource>,
}

impl InstructionDataset {
    /// Create a new instruction dataset.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        sources: Vec<InstructionSource>,
        instructions: Vec<InstructionEntry>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            version: version.into(),
            instructions,
            sources,
        }
    }

    fn source(&self, id: Uuid) -> Option<&InstructionSource> {
        self.sources.iter().find(|source| source.id == id)
    }
}

/// Tool descriptor for operational toolsets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTool {
    /// Tool name.
    pub name: String,
    /// Capability required for the tool.
    pub capability: String,
    /// Short description.
    pub description: String,
}

/// Skill module descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSkillModule {
    /// Skill name.
    pub name: String,
    /// Skill version.
    pub version: String,
    /// Short description.
    pub description: String,
}

/// A chat request that must be transformed into an operational payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Unique request identifier.
    pub request_id: Uuid,
    /// User-provided natural language message.
    pub message: String,
    /// Toolset available to the agent.
    pub toolset: Vec<ChatTool>,
    /// Skill modules available to the agent.
    pub skill_modules: Vec<ChatSkillModule>,
    /// Instruction dataset used to shape payload generation.
    pub instruction_dataset: InstructionDataset,
}

impl ChatRequest {
    /// Create a new chat request.
    pub fn new(
        message: impl Into<String>,
        toolset: Vec<ChatTool>,
        skill_modules: Vec<ChatSkillModule>,
        instruction_dataset: InstructionDataset,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            message: message.into(),
            toolset,
            skill_modules,
            instruction_dataset,
        }
    }
}

/// Instruction selection rules for chat payload generation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatInstructionSelection {
    /// Include instructions with any of these tags.
    pub tags: Vec<String>,
    /// Include instructions with explicit identifiers.
    pub instruction_ids: Vec<Uuid>,
}

impl ChatInstructionSelection {
    /// Return true if no selection filters are configured.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty() && self.instruction_ids.is_empty()
    }
}

/// Result of adapting a chat request into a structured payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAdaptation {
    /// Intent label used by flow control and orchestration.
    pub intent: String,
    /// Structured parameters for downstream execution.
    pub parameters: serde_json::Value,
}

/// Structured chat payload for operational flow control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatPayload {
    /// Request identifier.
    pub request_id: Uuid,
    /// Original user message.
    pub message: String,
    /// Intent label.
    pub intent: String,
    /// Structured parameters derived from the message.
    pub parameters: serde_json::Value,
    /// Toolset available to execute the request.
    pub toolset: Vec<ChatTool>,
    /// Skill modules available to execute the request.
    pub skill_modules: Vec<ChatSkillModule>,
    /// Instructions applied during payload generation.
    pub instructions: Vec<InstructionEntry>,
    /// Sources for the applied instructions.
    pub sources: Vec<InstructionSource>,
    /// Instruction dataset identifier.
    pub dataset_id: Uuid,
    /// Instruction dataset version.
    pub dataset_version: String,
}

/// Adapter that translates natural language into structured payloads.
pub trait ChatAdapter {
    /// Transform a request into an intent and parameter payload.
    fn adapt(
        &self,
        request: &ChatRequest,
        instructions: &[InstructionEntry],
    ) -> Result<ChatAdaptation, FlowControlError>;
}

/// Builds chat payloads and emits ActionLog events.
pub struct ChatPayloadBuilder<A> {
    adapter: A,
    action_log: Arc<ActionLog>,
}

impl<A> ChatPayloadBuilder<A>
where
    A: ChatAdapter,
{
    /// Create a new payload builder.
    pub fn new(adapter: A, action_log: Arc<ActionLog>) -> Self {
        Self {
            adapter,
            action_log,
        }
    }

    /// Build a chat payload from the request and selection rules.
    #[instrument(skip(self, request, selection), fields(request_id = %request.request_id))]
    pub fn build(
        &self,
        request: &ChatRequest,
        selection: &ChatInstructionSelection,
    ) -> Result<ChatPayload, FlowControlError> {
        self.action_log.append(ActionLogEntry::new(
            EventType::ChatRequestReceived,
            Actor::System,
            None,
            None,
            serde_json::json!({
                "request_id": request.request_id,
                "message": request.message,
                "tool_count": request.toolset.len(),
                "skill_count": request.skill_modules.len(),
                "dataset_id": request.instruction_dataset.id,
            }),
        ));

        let instructions = self.filter_instructions(request, selection);
        let sources = self.resolve_sources(&request.instruction_dataset, &instructions)?;

        let adaptation = self.adapter.adapt(request, &instructions)?;

        let payload = ChatPayload {
            request_id: request.request_id,
            message: request.message.clone(),
            intent: adaptation.intent.clone(),
            parameters: adaptation.parameters.clone(),
            toolset: request.toolset.clone(),
            skill_modules: request.skill_modules.clone(),
            instructions: instructions.clone(),
            sources: sources.clone(),
            dataset_id: request.instruction_dataset.id,
            dataset_version: request.instruction_dataset.version.clone(),
        };

        self.action_log.append(ActionLogEntry::new(
            EventType::ChatPayloadGenerated,
            Actor::System,
            None,
            None,
            serde_json::json!({
                "request_id": payload.request_id,
                "intent": payload.intent,
                "instruction_count": payload.instructions.len(),
                "source_count": payload.sources.len(),
                "tool_count": payload.toolset.len(),
                "skill_count": payload.skill_modules.len(),
            }),
        ));

        Ok(payload)
    }

    fn filter_instructions(
        &self,
        request: &ChatRequest,
        selection: &ChatInstructionSelection,
    ) -> Vec<InstructionEntry> {
        if selection.is_empty() {
            return request.instruction_dataset.instructions.clone();
        }

        request
            .instruction_dataset
            .instructions
            .iter()
            .filter(|instruction| {
                selection.instruction_ids.contains(&instruction.id)
                    || instruction
                        .tags
                        .iter()
                        .any(|tag| selection.tags.contains(tag))
            })
            .cloned()
            .collect()
    }

    fn resolve_sources(
        &self,
        dataset: &InstructionDataset,
        instructions: &[InstructionEntry],
    ) -> Result<Vec<InstructionSource>, FlowControlError> {
        let mut sources: HashMap<Uuid, InstructionSource> = HashMap::new();
        for instruction in instructions {
            let source = dataset.source(instruction.source_id).ok_or(
                FlowControlError::InstructionSourceMissing {
                    instruction_id: instruction.id,
                    source_id: instruction.source_id,
                },
            )?;
            sources.entry(source.id).or_insert_with(|| source.clone());
        }
        Ok(sources.into_values().collect())
    }
}

// ---------------------------------------------------------------------------
// ML model interface
// ---------------------------------------------------------------------------

/// Evaluates ML model scores used in flow control decisions.
pub trait ModelEvaluator {
    /// Return a model score for the supplied feature payload.
    fn score(&self, model: &str, features: &serde_json::Value) -> Result<f64, FlowControlError>;
}

// ---------------------------------------------------------------------------
// Flow state + engine
// ---------------------------------------------------------------------------

/// Tracks execution state for a running flow.
#[derive(Debug, Default)]
pub struct FlowState {
    visits: HashMap<Uuid, u32>,
}

impl FlowState {
    /// Create an empty flow state.
    pub fn new() -> Self {
        Self {
            visits: HashMap::new(),
        }
    }

    /// Record a visit to a step and return the updated count.
    pub fn record_visit(&mut self, step_id: Uuid) -> u32 {
        let entry = self.visits.entry(step_id).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Return the visit count for a given step.
    pub fn visits(&self, step_id: Uuid) -> u32 {
        self.visits.get(&step_id).copied().unwrap_or(0)
    }
}

/// Result of advancing a flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowAdvance {
    /// Move to the next step.
    Next(Uuid),
    /// Flow finished at a terminal step.
    Terminal(FlowTerminalStatus),
}

/// Evaluates flow control decisions using metrics and ML models.
pub struct FlowControlEngine<M> {
    model: M,
    action_log: Arc<ActionLog>,
}

impl<M> FlowControlEngine<M>
where
    M: ModelEvaluator,
{
    /// Create a new flow control engine.
    pub fn new(model: M, action_log: Arc<ActionLog>) -> Self {
        Self { model, action_log }
    }

    /// Advance the flow from the current step.
    ///
    /// Returns the next step or terminal status after evaluating transitions.
    #[instrument(skip(self, graph, context, state), fields(step_id = %step_id, task_id = %context.task_id))]
    pub fn advance(
        &self,
        graph: &FlowGraph,
        step_id: Uuid,
        context: &FlowContext,
        state: &mut FlowState,
    ) -> Result<FlowAdvance, FlowControlError> {
        let step = graph
            .step(step_id)
            .ok_or(FlowControlError::StepNotFound(step_id))?;

        let visit_count = state.record_visit(step_id);
        if let Some(limit) = step.max_visits {
            if visit_count > limit {
                let fallback = step
                    .fallback
                    .ok_or(FlowControlError::MaxVisitsExceeded { step_id, limit })?;
                self.ensure_step(graph, step_id, fallback)?;
                self.log_evaluated(
                    context,
                    step_id,
                    "max_visits_exceeded",
                    false,
                    serde_json::json!({ "limit": limit, "visits": visit_count }),
                    Some(fallback),
                );
                self.log_advanced(context, step_id, fallback);
                return Ok(FlowAdvance::Next(fallback));
            }
        }

        if let FlowStepKind::Terminal { status } = &step.kind {
            self.log_evaluated(
                context,
                step_id,
                "terminal",
                true,
                serde_json::json!({ "status": format!("{status:?}") }),
                None,
            );
            return Ok(FlowAdvance::Terminal(*status));
        }

        let mut matched: Option<(Uuid, String, serde_json::Value)> = None;
        for transition in &step.transitions {
            let (matched_transition, signal) =
                self.evaluate_condition(context, &transition.condition)?;
            if matched_transition {
                matched = Some((transition.target, transition.label.clone(), signal));
                break;
            }
        }

        let matched_transition = matched.is_some();
        let (next_step, decision, signal) = if let Some((target, label, signal)) = matched {
            (target, label, signal)
        } else if let Some(fallback) = step.fallback {
            (
                fallback,
                "fallback".to_owned(),
                serde_json::json!({ "reason": "no_transition_matched" }),
            )
        } else {
            return Err(FlowControlError::NoMatchingTransition(step_id));
        };

        self.ensure_step(graph, step_id, next_step)?;

        debug!(from = %step_id, to = %next_step, %decision, "flow advanced");
        self.log_evaluated(
            context,
            step_id,
            &decision,
            matched_transition,
            signal,
            Some(next_step),
        );
        self.log_advanced(context, step_id, next_step);

        Ok(FlowAdvance::Next(next_step))
    }

    fn ensure_step(
        &self,
        graph: &FlowGraph,
        from: Uuid,
        target: Uuid,
    ) -> Result<(), FlowControlError> {
        if graph.step(target).is_none() {
            return Err(FlowControlError::TransitionTargetNotFound { from, target });
        }
        Ok(())
    }

    fn evaluate_condition(
        &self,
        context: &FlowContext,
        condition: &FlowCondition,
    ) -> Result<(bool, serde_json::Value), FlowControlError> {
        match condition {
            FlowCondition::Always => Ok((true, serde_json::json!({ "kind": "always" }))),
            FlowCondition::MetricAtLeast { metric, threshold } => {
                let value = context
                    .metrics
                    .get(metric)
                    .copied()
                    .ok_or_else(|| FlowControlError::MissingMetric(metric.clone()))?;
                Ok((
                    value >= *threshold,
                    serde_json::json!({
                        "kind": "metric_at_least",
                        "metric": metric,
                        "value": value,
                        "threshold": threshold,
                    }),
                ))
            }
            FlowCondition::MetricBelow { metric, threshold } => {
                let value = context
                    .metrics
                    .get(metric)
                    .copied()
                    .ok_or_else(|| FlowControlError::MissingMetric(metric.clone()))?;
                Ok((
                    value < *threshold,
                    serde_json::json!({
                        "kind": "metric_below",
                        "metric": metric,
                        "value": value,
                        "threshold": threshold,
                    }),
                ))
            }
            FlowCondition::ModelScoreAtLeast { model, threshold } => {
                let score = self.model.score(model, &context.features)?;
                Ok((
                    score >= *threshold,
                    serde_json::json!({
                        "kind": "model_score_at_least",
                        "model": model,
                        "score": score,
                        "threshold": threshold,
                    }),
                ))
            }
        }
    }

    fn log_evaluated(
        &self,
        context: &FlowContext,
        step_id: Uuid,
        decision: &str,
        matched: bool,
        signal: serde_json::Value,
        target: Option<Uuid>,
    ) {
        self.action_log.append(ActionLogEntry::new(
            EventType::FlowEvaluated,
            Actor::System,
            Some(context.dimension_id),
            Some(context.task_id),
            serde_json::json!({
                "step_id": step_id,
                "decision": decision,
                "matched": matched,
                "signal": signal,
                "target_step": target,
            }),
        ));
    }

    fn log_advanced(&self, context: &FlowContext, from: Uuid, to: Uuid) {
        self.action_log.append(ActionLogEntry::new(
            EventType::FlowAdvanced,
            Actor::System,
            Some(context.dimension_id),
            Some(context.task_id),
            serde_json::json!({ "from": from, "to": to }),
        ));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct StubModel {
        scores: HashMap<String, f64>,
    }

    impl StubModel {
        fn new(scores: HashMap<String, f64>) -> Self {
            Self { scores }
        }
    }

    impl ModelEvaluator for StubModel {
        fn score(
            &self,
            model: &str,
            _features: &serde_json::Value,
        ) -> Result<f64, FlowControlError> {
            self.scores
                .get(model)
                .copied()
                .ok_or_else(|| FlowControlError::ModelEvaluationFailed {
                    model: model.to_owned(),
                    reason: "model not found".to_owned(),
                })
        }
    }

    fn setup_graph() -> (FlowGraph, Uuid, Uuid, Uuid) {
        let mut start = FlowStep::decision("decide", "choose path");
        let next_a = FlowStep::task("task-a", Uuid::new_v4());
        let next_b = FlowStep::task("task-b", Uuid::new_v4());
        let next_a_id = next_a.id;
        let next_b_id = next_b.id;

        start.add_transition(FlowTransition::new(
            "score",
            FlowCondition::MetricAtLeast {
                metric: "score".to_owned(),
                threshold: 0.75,
            },
            next_a_id,
        ));
        start.fallback = Some(next_b_id);

        let mut graph = FlowGraph::new("test", start);
        graph.insert_step(next_a);
        graph.insert_step(next_b);

        let entrypoint = graph.entrypoint;
        (graph, entrypoint, next_a_id, next_b_id)
    }

    struct StubChatAdapter {
        intent: String,
    }

    impl ChatAdapter for StubChatAdapter {
        fn adapt(
            &self,
            request: &ChatRequest,
            instructions: &[InstructionEntry],
        ) -> Result<ChatAdaptation, FlowControlError> {
            Ok(ChatAdaptation {
                intent: self.intent.clone(),
                parameters: serde_json::json!({
                    "message": request.message,
                    "instruction_count": instructions.len(),
                }),
            })
        }
    }

    #[test]
    fn advance_selects_metric_transition() {
        let (graph, start_id, next_a_id, _) = setup_graph();
        let log = ActionLog::new(8);
        let engine = FlowControlEngine::new(StubModel::new(HashMap::new()), log);

        let mut context = FlowContext::new(DimensionId::new(), TaskId::new());
        context.metrics.insert("score".to_owned(), 0.9);

        let mut state = FlowState::new();
        let result = engine
            .advance(&graph, start_id, &context, &mut state)
            .unwrap();
        assert_eq!(result, FlowAdvance::Next(next_a_id));
    }

    #[test]
    fn advance_uses_model_score() {
        let mut start = FlowStep::decision("model", "ml path");
        let high = FlowStep::task("high", Uuid::new_v4());
        let low = FlowStep::task("low", Uuid::new_v4());
        let high_id = high.id;
        let low_id = low.id;

        start.add_transition(FlowTransition::new(
            "ml-score",
            FlowCondition::ModelScoreAtLeast {
                model: "gate".to_owned(),
                threshold: 0.5,
            },
            high_id,
        ));
        start.fallback = Some(low_id);

        let mut graph = FlowGraph::new("ml", start);
        graph.insert_step(high);
        graph.insert_step(low);

        let mut scores = HashMap::new();
        scores.insert("gate".to_owned(), 0.7);
        let log = ActionLog::new(8);
        let engine = FlowControlEngine::new(StubModel::new(scores), log);

        let context = FlowContext::new(DimensionId::new(), TaskId::new());
        let mut state = FlowState::new();

        let result = engine
            .advance(&graph, graph.entrypoint, &context, &mut state)
            .unwrap();
        assert_eq!(result, FlowAdvance::Next(high_id));
    }

    #[test]
    fn advance_falls_back_when_max_visits_exceeded() {
        let mut start = FlowStep::decision("loop", "bounded loop").with_max_visits(1);
        let next = FlowStep::task("next", Uuid::new_v4());
        let fallback = FlowStep::task("fallback", Uuid::new_v4());
        let next_id = next.id;
        let fallback_id = fallback.id;

        start.add_transition(FlowTransition::new(
            "always",
            FlowCondition::Always,
            next_id,
        ));
        start.fallback = Some(fallback_id);

        let mut graph = FlowGraph::new("loop", start);
        graph.insert_step(next);
        graph.insert_step(fallback);

        let log = ActionLog::new(8);
        let engine = FlowControlEngine::new(StubModel::new(HashMap::new()), log);
        let context = FlowContext::new(DimensionId::new(), TaskId::new());
        let mut state = FlowState::new();

        let first = engine
            .advance(&graph, graph.entrypoint, &context, &mut state)
            .unwrap();
        assert_eq!(first, FlowAdvance::Next(next_id));

        let second = engine
            .advance(&graph, graph.entrypoint, &context, &mut state)
            .unwrap();
        assert_eq!(second, FlowAdvance::Next(fallback_id));
    }

    #[test]
    fn chat_payload_builder_selects_instructions_and_sources() {
        let source_a = InstructionSource::new("spec", None, "Ops spec");
        let source_b = InstructionSource::new("dataset", None, "Dataset");
        let instruction_a =
            InstructionEntry::new("Prefer safe defaults", vec!["safety".into()], source_a.id);
        let instruction_b =
            InstructionEntry::new("Use high throughput", vec!["perf".into()], source_b.id);

        let dataset = InstructionDataset::new(
            "ops",
            "v1",
            vec![source_a.clone(), source_b.clone()],
            vec![instruction_a.clone(), instruction_b.clone()],
        );

        let request = ChatRequest::new(
            "Plan a workflow",
            vec![ChatTool {
                name: "mesh".to_owned(),
                capability: "artifact.read".to_owned(),
                description: "Mesh access".to_owned(),
            }],
            vec![ChatSkillModule {
                name: "planner".to_owned(),
                version: "1.0".to_owned(),
                description: "Planning skill".to_owned(),
            }],
            dataset,
        );

        let selection = ChatInstructionSelection {
            tags: vec!["safety".into()],
            instruction_ids: vec![],
        };

        let log = ActionLog::new(8);
        let builder = ChatPayloadBuilder::new(
            StubChatAdapter {
                intent: "workflow.plan".to_owned(),
            },
            Arc::clone(&log),
        );

        let payload = builder.build(&request, &selection).unwrap();
        assert_eq!(payload.instructions.len(), 1);
        assert_eq!(payload.sources.len(), 1);
        assert_eq!(payload.intent, "workflow.plan");

        let events = log.all_entries();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, EventType::ChatRequestReceived);
        assert_eq!(events[1].event_type, EventType::ChatPayloadGenerated);
    }
}
