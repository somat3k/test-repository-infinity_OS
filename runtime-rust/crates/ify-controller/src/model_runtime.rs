//! Model runtime coordination for dynamic tuning and replica pooling.
//!
//! This module begins Epic H by introducing:
//! - Performance-driven hyperparameter adjustments.
//! - Quick reload requests when live metrics degrade.
//! - Replica pooling for multi-model (ML + AI) execution stacks.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info};

use crate::action_log::{now_ms, ActionLog, ActionLogEntry, Actor, EventType};
use crate::flow_control::ComparisonOperator;

const COMPARISON_TOLERANCE: f64 = 1e-9;

/// Supported model kinds for runtime coordination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    /// Classic machine learning model.
    Ml,
    /// AI model (foundation / reasoning).
    Ai,
}

/// Hyperparameter map associated with a model instance.
pub type Hyperparameters = HashMap<String, serde_json::Value>;

/// A performance sample captured from a live model execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSample {
    /// Unix epoch milliseconds when the sample was captured.
    pub captured_at_ms: u64,
    /// Named metrics (latency, accuracy, cost, etc.).
    pub metrics: HashMap<String, f64>,
}

impl PerformanceSample {
    /// Create a new sample using the current timestamp.
    pub fn new(metrics: HashMap<String, f64>) -> Self {
        Self {
            captured_at_ms: now_ms(),
            metrics,
        }
    }

    /// Create a new sample with an explicit timestamp.
    pub fn with_timestamp(captured_at_ms: u64, metrics: HashMap<String, f64>) -> Self {
        Self {
            captured_at_ms,
            metrics,
        }
    }
}

/// Hyperparameter change instructions derived from a rule evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperparameterAdjustment {
    /// Model identifier that the adjustment applies to.
    pub model_id: String,
    /// Metric that triggered the adjustment.
    pub metric: String,
    /// Observed metric value.
    pub observed: f64,
    /// Threshold from the rule.
    pub threshold: f64,
    /// Comparison operator used to match the rule.
    pub operator: ComparisonOperator,
    /// Human-readable reason.
    pub reason: String,
    /// Parameter updates to apply.
    pub changes: Hyperparameters,
}

/// Rule for tuning hyperparameters based on a metric signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperparameterRule {
    /// Metric to evaluate.
    pub metric: String,
    /// Comparison operator.
    pub operator: ComparisonOperator,
    /// Threshold value.
    pub threshold: f64,
    /// Parameter changes to apply when the rule matches.
    pub changes: Hyperparameters,
    /// Human-readable reason.
    pub reason: String,
}

impl HyperparameterRule {
    fn evaluate(
        &self,
        model_id: &str,
        sample: &PerformanceSample,
    ) -> Result<Option<HyperparameterAdjustment>, ModelRuntimeError> {
        let value = sample
            .metrics
            .get(&self.metric)
            .copied()
            .ok_or_else(|| ModelRuntimeError::MissingMetric(self.metric.clone()))?;
        let matched = compare_metric(value, self.operator, self.threshold);
        if matched {
            Ok(Some(HyperparameterAdjustment {
                model_id: model_id.to_owned(),
                metric: self.metric.clone(),
                observed: value,
                threshold: self.threshold,
                operator: self.operator,
                reason: self.reason.clone(),
                changes: self.changes.clone(),
            }))
        } else {
            Ok(None)
        }
    }
}

/// Tuning policy that evaluates performance samples.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HyperparameterPolicy {
    /// Ordered list of tuning rules to evaluate.
    pub rules: Vec<HyperparameterRule>,
}

/// Reload policy for model instances when performance degrades.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReloadPolicy {
    /// Metric to evaluate for reload triggers.
    pub metric: String,
    /// Comparison operator.
    pub operator: ComparisonOperator,
    /// Threshold value.
    pub threshold: f64,
    /// Minimum number of samples required before reload can trigger.
    pub min_samples: u64,
    /// Minimum time between reloads.
    pub cooldown_ms: u64,
}

impl ModelReloadPolicy {
    /// Create a policy with immediate reloads and no cooldown.
    pub fn immediate(metric: impl Into<String>, operator: ComparisonOperator, threshold: f64) -> Self {
        Self {
            metric: metric.into(),
            operator,
            threshold,
            min_samples: 1,
            cooldown_ms: 0,
        }
    }
}

/// Model reload request emitted after evaluating performance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReloadRequest {
    /// Model identifier to reload.
    pub model_id: String,
    /// Reload generation counter.
    pub reload_generation: u64,
    /// Observed metric value.
    pub observed: f64,
    /// Threshold value.
    pub threshold: f64,
    /// Metric used for evaluation.
    pub metric: String,
    /// Human-readable reason.
    pub reason: String,
    /// Timestamp of the reload request.
    pub requested_at_ms: u64,
}

/// Outcome of a performance evaluation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelOptimizationDecision {
    /// Hyperparameter adjustments to apply.
    pub adjustments: Vec<HyperparameterAdjustment>,
    /// Reload request, if any.
    pub reload: Option<ModelReloadRequest>,
}

/// Static model registration data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Unique model identifier.
    pub model_id: String,
    /// Dimension that owns the model.
    pub dimension_id: DimensionId,
    /// Task that registered or owns the model.
    pub task_id: TaskId,
    /// Model kind (ML vs AI).
    pub kind: ModelKind,
    /// Version identifier for the model.
    pub version: String,
    /// Current hyperparameter configuration.
    pub hyperparameters: Hyperparameters,
    /// Tuning policy for dynamic adjustments.
    pub tuning_policy: HyperparameterPolicy,
    /// Reload policy for quick recovery.
    pub reload_policy: ModelReloadPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelState {
    profile: ModelProfile,
    sample_count: u64,
    reload_generation: u64,
    last_reload_at_ms: Option<u64>,
}

/// Errors produced by the model runtime components.
#[derive(Debug, Error)]
pub enum ModelRuntimeError {
    /// The model identifier is not registered.
    #[error("model '{0}' is not registered")]
    ModelNotFound(String),
    /// The model is already registered.
    #[error("model '{0}' is already registered")]
    ModelAlreadyRegistered(String),
    /// A required metric was missing from the sample.
    #[error("metric '{0}' missing from performance sample")]
    MissingMetric(String),
}

/// Coordinates performance-driven tuning and reloads.
pub struct ModelPerformanceManager {
    models: Mutex<HashMap<String, ModelState>>,
    action_log: Arc<ActionLog>,
}

impl ModelPerformanceManager {
    /// Create a new performance manager.
    pub fn new(action_log: Arc<ActionLog>) -> Self {
        Self {
            models: Mutex::new(HashMap::new()),
            action_log,
        }
    }

    /// Register a model profile for performance tracking.
    pub fn register_model(&self, profile: ModelProfile) -> Result<(), ModelRuntimeError> {
        let mut guard = self.models.lock().expect("model manager lock poisoned");
        if guard.contains_key(&profile.model_id) {
            return Err(ModelRuntimeError::ModelAlreadyRegistered(
                profile.model_id.clone(),
            ));
        }
        guard.insert(
            profile.model_id.clone(),
            ModelState {
                profile,
                sample_count: 0,
                reload_generation: 0,
                last_reload_at_ms: None,
            },
        );
        Ok(())
    }

    /// Record a performance sample and return any tuning or reload decisions.
    pub fn record_performance(
        &self,
        model_id: &str,
        sample: PerformanceSample,
    ) -> Result<ModelOptimizationDecision, ModelRuntimeError> {
        let mut guard = self.models.lock().expect("model manager lock poisoned");
        let state = guard
            .get_mut(model_id)
            .ok_or_else(|| ModelRuntimeError::ModelNotFound(model_id.to_owned()))?;
        state.sample_count += 1;

        let mut decision = ModelOptimizationDecision::default();
        let rules = state.profile.tuning_policy.rules.clone();
        for rule in rules {
            if let Some(adjustment) = rule.evaluate(model_id, &sample)? {
                Self::apply_adjustment(state, &adjustment);
                self.log_hyperparameter_adjustment(state, &adjustment);
                decision.adjustments.push(adjustment);
            }
        }

        if let Some(reload) = Self::evaluate_reload(state, &sample)? {
            self.log_reload(state, &reload);
            decision.reload = Some(reload);
        }

        Ok(decision)
    }

    /// Retrieve a snapshot of a registered model profile.
    pub fn snapshot(&self, model_id: &str) -> Result<ModelProfile, ModelRuntimeError> {
        let guard = self.models.lock().expect("model manager lock poisoned");
        let state = guard
            .get(model_id)
            .ok_or_else(|| ModelRuntimeError::ModelNotFound(model_id.to_owned()))?;
        Ok(state.profile.clone())
    }

    fn apply_adjustment(state: &mut ModelState, adjustment: &HyperparameterAdjustment) {
        for (key, value) in &adjustment.changes {
            state.profile.hyperparameters.insert(key.clone(), value.clone());
        }
    }

    fn evaluate_reload(
        state: &mut ModelState,
        sample: &PerformanceSample,
    ) -> Result<Option<ModelReloadRequest>, ModelRuntimeError> {
        let policy = &state.profile.reload_policy;
        if state.sample_count < policy.min_samples {
            return Ok(None);
        }
        let value = sample
            .metrics
            .get(&policy.metric)
            .copied()
            .ok_or_else(|| ModelRuntimeError::MissingMetric(policy.metric.clone()))?;
        let matches = compare_metric(value, policy.operator, policy.threshold);
        if !matches {
            return Ok(None);
        }
        if let Some(last) = state.last_reload_at_ms {
            if sample.captured_at_ms.saturating_sub(last) < policy.cooldown_ms {
                return Ok(None);
            }
        }
        state.reload_generation += 1;
        state.last_reload_at_ms = Some(sample.captured_at_ms);
        Ok(Some(ModelReloadRequest {
            model_id: state.profile.model_id.clone(),
            reload_generation: state.reload_generation,
            observed: value,
            threshold: policy.threshold,
            metric: policy.metric.clone(),
            reason: "performance threshold breached".to_owned(),
            requested_at_ms: sample.captured_at_ms,
        }))
    }

    fn log_hyperparameter_adjustment(
        &self,
        state: &ModelState,
        adjustment: &HyperparameterAdjustment,
    ) {
        info!(
            model = %state.profile.model_id,
            metric = %adjustment.metric,
            "hyperparameter adjustment triggered"
        );
        self.action_log.append(ActionLogEntry::new(
            EventType::ModelHyperparametersAdjusted,
            Actor::System,
            Some(state.profile.dimension_id),
            Some(state.profile.task_id),
            serde_json::json!({
                "model_id": adjustment.model_id,
                "version": state.profile.version,
                "metric": adjustment.metric,
                "observed": adjustment.observed,
                "threshold": adjustment.threshold,
                "operator": format!("{:?}", adjustment.operator),
                "reason": adjustment.reason,
                "changes": adjustment.changes,
            }),
        ));
    }

    fn log_reload(&self, state: &ModelState, reload: &ModelReloadRequest) {
        info!(
            model = %state.profile.model_id,
            generation = reload.reload_generation,
            "model reload requested"
        );
        self.action_log.append(ActionLogEntry::new(
            EventType::ModelReloadRequested,
            Actor::System,
            Some(state.profile.dimension_id),
            Some(state.profile.task_id),
            serde_json::json!({
                "model_id": reload.model_id,
                "version": state.profile.version,
                "metric": reload.metric,
                "observed": reload.observed,
                "threshold": reload.threshold,
                "reason": reload.reason,
                "reload_generation": reload.reload_generation,
                "requested_at_ms": reload.requested_at_ms,
            }),
        ));
    }
}

/// Identifier for a kernel replica.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReplicaId(u32);

impl ReplicaId {
    /// Create a new replica identifier from a raw value.
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the raw numeric value of this replica identifier.
    pub fn value(self) -> u32 {
        self.0
    }
}

/// Kernel replica policy applied to model instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaPolicy {
    /// Maximum tasks allowed in the replica scheduler.
    pub max_tasks: u32,
    /// Memory cap for the replica arena in bytes.
    pub arena_cap_bytes: u64,
    /// Whether the replica auto-destroys when idle.
    pub auto_destroy: bool,
    /// Optional CPU pin for the replica scheduler.
    pub cpu_pin: Option<u8>,
}

impl Default for ReplicaPolicy {
    fn default() -> Self {
        Self {
            max_tasks: 32,
            arena_cap_bytes: 1 << 20,
            auto_destroy: true,
            cpu_pin: None,
        }
    }
}

/// Request to create a kernel replica for a model module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaRequest {
    /// Owning dimension.
    pub dimension_id: DimensionId,
    /// Task that requested the replica.
    pub task_id: TaskId,
    /// Model identifier.
    pub model_id: String,
    /// Module identifier.
    pub module_id: String,
    /// Replica policy to apply.
    pub policy: ReplicaPolicy,
}

/// Replica handle returned by a kernel implementation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaHandle {
    /// Replica identifier.
    pub id: ReplicaId,
    /// Original request.
    pub request: ReplicaRequest,
    /// Timestamp when the replica was created.
    pub created_at_ms: u64,
}

/// Errors reported by a replica kernel implementation.
#[derive(Debug, Error)]
pub enum ReplicaKernelError {
    /// The replica limit has been reached.
    #[error("replica limit reached ({0})")]
    LimitReached(u32),
    /// Replica not found.
    #[error("replica {0} not found")]
    ReplicaNotFound(u32),
}

/// Kernel replica interface used by the model pool.
pub trait ReplicaKernel: Send + Sync {
    /// Create a replica for the provided request.
    fn create_replica(&self, request: &ReplicaRequest) -> Result<ReplicaHandle, ReplicaKernelError>;
    /// Destroy the replica with the specified identifier.
    fn destroy_replica(&self, id: ReplicaId) -> Result<(), ReplicaKernelError>;
}

/// In-memory replica kernel for tests and local execution.
pub struct InMemoryReplicaKernel {
    max_replicas: u32,
    next_id: AtomicU32,
    replicas: Mutex<HashMap<ReplicaId, ReplicaHandle>>,
}

impl InMemoryReplicaKernel {
    /// Create a new in-memory kernel with the specified replica limit.
    pub fn new(max_replicas: u32) -> Self {
        Self {
            max_replicas,
            next_id: AtomicU32::new(1),
            replicas: Mutex::new(HashMap::new()),
        }
    }
}

impl ReplicaKernel for InMemoryReplicaKernel {
    fn create_replica(&self, request: &ReplicaRequest) -> Result<ReplicaHandle, ReplicaKernelError> {
        let mut guard = self.replicas.lock().expect("replica kernel lock poisoned");
        if guard.len() as u32 >= self.max_replicas {
            return Err(ReplicaKernelError::LimitReached(self.max_replicas));
        }
        let id = ReplicaId::new(self.next_id.fetch_add(1, Ordering::Relaxed));
        let handle = ReplicaHandle {
            id,
            request: request.clone(),
            created_at_ms: now_ms(),
        };
        guard.insert(id, handle.clone());
        Ok(handle)
    }

    fn destroy_replica(&self, id: ReplicaId) -> Result<(), ReplicaKernelError> {
        let mut guard = self.replicas.lock().expect("replica kernel lock poisoned");
        if guard.remove(&id).is_some() {
            Ok(())
        } else {
            Err(ReplicaKernelError::ReplicaNotFound(id.value()))
        }
    }
}

/// Module metadata used to align model ensembles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelModule {
    /// Module identifier.
    pub module_id: String,
    /// Model kind.
    pub kind: ModelKind,
    /// Efficiency score (higher is better).
    pub efficiency_score: f64,
    /// Complexity handling weight (higher means better at complex tasks).
    pub complexity_weight: f64,
    /// Maximum replicas allowed for this module.
    pub max_replicas: u32,
}

/// Demand signal for provisioning replicas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaDemand {
    /// Dimension for the replicas.
    pub dimension_id: DimensionId,
    /// Task requesting replicas.
    pub task_id: TaskId,
    /// Target model identifier.
    pub model_id: String,
    /// Complexity score (0.0–1.0).
    pub complexity: f64,
    /// Efficiency target (0.0–1.0).
    pub efficiency_target: f64,
    /// Maximum replicas to provision.
    pub max_replicas: u32,
    /// Replica policy for all created replicas.
    pub policy: ReplicaPolicy,
}

/// Selected module with replica allocations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelModuleSelection {
    /// Module identifier.
    pub module_id: String,
    /// Model kind.
    pub kind: ModelKind,
    /// Alignment score used for ranking.
    pub alignment_score: f64,
    /// Number of replicas allocated.
    pub replicas: u32,
}

/// Plan for a multi-model ensemble.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEnsemblePlan {
    /// Model identifier this plan targets.
    pub model_id: String,
    /// Selected modules with replica allocation.
    pub modules: Vec<ModelModuleSelection>,
    /// Total replicas requested.
    pub total_replicas: u32,
    /// Reasoning for the selection.
    pub rationale: String,
}

/// Provisioned replica metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaProvisioning {
    /// Ensemble plan used for provisioning.
    pub plan: ModelEnsemblePlan,
    /// Replica handles created for this plan.
    pub replicas: Vec<ReplicaHandle>,
}

/// Errors produced by the replica pool.
#[derive(Debug, Error)]
pub enum ModelReplicaError {
    /// No modules were registered for selection.
    #[error("no model modules registered for replica planning")]
    NoModulesAvailable,
    /// Kernel reported an error.
    #[error("replica kernel error: {0}")]
    KernelError(#[from] ReplicaKernelError),
}

/// Pool that aligns modules and provisions replicas using kernel interfaces.
pub struct ModelReplicaPool<K: ReplicaKernel> {
    kernel: K,
    action_log: Arc<ActionLog>,
    modules: Mutex<HashMap<String, ModelModule>>,
    replicas: Mutex<HashMap<ReplicaId, ReplicaHandle>>,
}

impl<K: ReplicaKernel> ModelReplicaPool<K> {
    /// Create a new replica pool.
    pub fn new(kernel: K, action_log: Arc<ActionLog>) -> Self {
        Self {
            kernel,
            action_log,
            modules: Mutex::new(HashMap::new()),
            replicas: Mutex::new(HashMap::new()),
        }
    }

    /// Register a module for replica planning.
    pub fn register_module(&self, module: ModelModule) {
        let mut guard = self.modules.lock().expect("replica pool lock poisoned");
        guard.insert(module.module_id.clone(), module);
    }

    /// Align modules and provision replicas for the given demand signal.
    pub fn provision(&self, demand: ReplicaDemand) -> Result<ReplicaProvisioning, ModelReplicaError> {
        let plan = self.plan_modules(&demand)?;
        let mut replicas = Vec::new();
        for selection in &plan.modules {
            for _ in 0..selection.replicas {
                let request = ReplicaRequest {
                    dimension_id: demand.dimension_id,
                    task_id: demand.task_id,
                    model_id: demand.model_id.clone(),
                    module_id: selection.module_id.clone(),
                    policy: demand.policy.clone(),
                };
                let handle = self.kernel.create_replica(&request)?;
                self.replicas
                    .lock()
                    .expect("replica pool lock poisoned")
                    .insert(handle.id, handle.clone());
                self.log_replica_provisioned(&demand, &handle, selection);
                replicas.push(handle);
            }
        }
        Ok(ReplicaProvisioning { plan, replicas })
    }

    /// Release a replica by identifier.
    pub fn release(&self, id: ReplicaId) -> Result<(), ModelReplicaError> {
        let handle = {
            let mut guard = self.replicas.lock().expect("replica pool lock poisoned");
            guard.remove(&id)
        };
        if let Some(handle) = handle {
            self.kernel.destroy_replica(id)?;
            self.log_replica_released(&handle);
        }
        Ok(())
    }

    fn plan_modules(&self, demand: &ReplicaDemand) -> Result<ModelEnsemblePlan, ModelReplicaError> {
        let modules = self.modules.lock().expect("replica pool lock poisoned");
        if modules.is_empty() {
            return Err(ModelReplicaError::NoModulesAvailable);
        }
        let complexity = demand.complexity.clamp(0.0, 1.0);
        let efficiency_weight = demand.efficiency_target.clamp(0.0, 1.0);
        let complexity_weight = 1.0 - efficiency_weight;

        let mut scored: Vec<(ModelModule, f64)> = modules
            .values()
            .cloned()
            .map(|module| {
                let score =
                    module.efficiency_score * efficiency_weight + module.complexity_weight * complexity_weight;
                (module, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let needs_ensemble = complexity >= 0.7;
        let mut selections = Vec::new();
        if needs_ensemble {
            let mut best_ml: Option<(ModelModule, f64)> = None;
            let mut best_ai: Option<(ModelModule, f64)> = None;
            for (module, score) in &scored {
                match module.kind {
                    ModelKind::Ml if best_ml.is_none() => {
                        best_ml = Some((module.clone(), *score));
                    }
                    ModelKind::Ai if best_ai.is_none() => {
                        best_ai = Some((module.clone(), *score));
                    }
                    _ => {}
                }
                if best_ml.is_some() && best_ai.is_some() {
                    break;
                }
            }
            if let (Some((ml, ml_score)), Some((ai, ai_score))) = (best_ml, best_ai) {
                selections.push((ml.clone(), ml_score));
                if ml.module_id != ai.module_id {
                    selections.push((ai.clone(), ai_score));
                }
            } else if let Some((module, score)) = scored.first() {
                selections.push((module.clone(), *score));
            }
        } else if let Some((module, score)) = scored.first() {
            selections.push((module.clone(), *score));
        }

        let base_count = selections.len().max(1) as u32;
        let desired_total = ((demand.max_replicas as f64) * complexity)
            .ceil()
            .max(base_count as f64) as u32;
        let mut remaining = desired_total.max(base_count);

        let mut planned_modules = Vec::new();
        for (index, (module, score)) in selections.iter().enumerate() {
            let mut replicas = 1;
            if index == 0 && remaining > base_count {
                replicas += remaining - base_count;
            }
            replicas = replicas.max(1).min(module.max_replicas.max(1));
            planned_modules.push(ModelModuleSelection {
                module_id: module.module_id.clone(),
                kind: module.kind,
                alignment_score: *score,
                replicas,
            });
            remaining = remaining.saturating_sub(replicas);
        }

        let total_replicas = planned_modules.iter().map(|m| m.replicas).sum();
        Ok(ModelEnsemblePlan {
            model_id: demand.model_id.clone(),
            modules: planned_modules,
            total_replicas,
            rationale: if needs_ensemble {
                "high complexity signal requested an ML+AI ensemble".to_owned()
            } else {
                "efficiency-weighted primary module selected".to_owned()
            },
        })
    }

    fn log_replica_provisioned(
        &self,
        demand: &ReplicaDemand,
        handle: &ReplicaHandle,
        selection: &ModelModuleSelection,
    ) {
        debug!(
            replica_id = handle.id.value(),
            model = %demand.model_id,
            module = %selection.module_id,
            "replica provisioned"
        );
        self.action_log.append(ActionLogEntry::new(
            EventType::ReplicaProvisioned,
            Actor::Kernel,
            Some(demand.dimension_id),
            Some(demand.task_id),
            serde_json::json!({
                "replica_id": handle.id.value(),
                "model_id": demand.model_id,
                "module_id": selection.module_id,
                "kind": format!("{:?}", selection.kind),
                "complexity": demand.complexity,
                "efficiency_target": demand.efficiency_target,
            }),
        ));
    }

    fn log_replica_released(&self, handle: &ReplicaHandle) {
        info!(
            replica_id = handle.id.value(),
            model = %handle.request.model_id,
            "replica released"
        );
        self.action_log.append(ActionLogEntry::new(
            EventType::ReplicaReleased,
            Actor::Kernel,
            Some(handle.request.dimension_id),
            Some(handle.request.task_id),
            serde_json::json!({
                "replica_id": handle.id.value(),
                "model_id": handle.request.model_id,
                "module_id": handle.request.module_id,
            }),
        ));
    }
}

fn compare_metric(value: f64, operator: ComparisonOperator, threshold: f64) -> bool {
    match operator {
        ComparisonOperator::AtLeast => value >= threshold,
        ComparisonOperator::AtMost => value <= threshold,
        ComparisonOperator::Equal => {
            let diff = (value - threshold).abs();
            let scale = value.abs().max(threshold.abs()).max(1.0);
            diff <= scale * COMPARISON_TOLERANCE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_log::EventType;

    #[test]
    fn performance_manager_adjusts_and_reloads() {
        let log = ActionLog::new(16);
        let manager = ModelPerformanceManager::new(Arc::clone(&log));

        let mut hyperparameters = Hyperparameters::new();
        hyperparameters.insert("learning_rate".to_owned(), serde_json::json!(0.01));

        let rule = HyperparameterRule {
            metric: "latency_ms".to_owned(),
            operator: ComparisonOperator::AtLeast,
            threshold: 200.0,
            changes: [("learning_rate".to_owned(), serde_json::json!(0.005))]
                .into_iter()
                .collect(),
            reason: "latency too high".to_owned(),
        };

        let profile = ModelProfile {
            model_id: "model-a".to_owned(),
            dimension_id: DimensionId::new(),
            task_id: TaskId::new(),
            kind: ModelKind::Ml,
            version: "v1".to_owned(),
            hyperparameters,
            tuning_policy: HyperparameterPolicy { rules: vec![rule] },
            reload_policy: ModelReloadPolicy::immediate(
                "quality_score",
                ComparisonOperator::AtMost,
                0.8,
            ),
        };

        manager.register_model(profile).unwrap();
        let sample = PerformanceSample::with_timestamp(
            10,
            [("latency_ms".to_owned(), 240.0), ("quality_score".to_owned(), 0.7)]
                .into_iter()
                .collect(),
        );
        let decision = manager.record_performance("model-a", sample).unwrap();
        assert_eq!(decision.adjustments.len(), 1);
        assert!(decision.reload.is_some());

        let snapshot = manager.snapshot("model-a").unwrap();
        assert_eq!(
            snapshot.hyperparameters.get("learning_rate"),
            Some(&serde_json::json!(0.005))
        );

        let entries = log.all_entries();
        let event_types: Vec<EventType> = entries.into_iter().map(|e| e.event_type).collect();
        assert!(event_types.contains(&EventType::ModelHyperparametersAdjusted));
        assert!(event_types.contains(&EventType::ModelReloadRequested));
    }

    #[test]
    fn reload_respects_cooldown() {
        let log = ActionLog::new(8);
        let manager = ModelPerformanceManager::new(Arc::clone(&log));

        let profile = ModelProfile {
            model_id: "model-b".to_owned(),
            dimension_id: DimensionId::new(),
            task_id: TaskId::new(),
            kind: ModelKind::Ai,
            version: "v2".to_owned(),
            hyperparameters: Hyperparameters::new(),
            tuning_policy: HyperparameterPolicy::default(),
            reload_policy: ModelReloadPolicy {
                metric: "accuracy".to_owned(),
                operator: ComparisonOperator::AtMost,
                threshold: 0.9,
                min_samples: 1,
                cooldown_ms: 100,
            },
        };

        manager.register_model(profile).unwrap();
        let first = PerformanceSample::with_timestamp(
            1000,
            [("accuracy".to_owned(), 0.85)].into_iter().collect(),
        );
        let decision = manager.record_performance("model-b", first).unwrap();
        assert!(decision.reload.is_some());

        let second = PerformanceSample::with_timestamp(
            1050,
            [("accuracy".to_owned(), 0.8)].into_iter().collect(),
        );
        let decision = manager.record_performance("model-b", second).unwrap();
        assert!(decision.reload.is_none());
    }

    #[test]
    fn replica_pool_provisions_ensemble() {
        let log = ActionLog::new(16);
        let kernel = InMemoryReplicaKernel::new(4);
        let pool = ModelReplicaPool::new(kernel, Arc::clone(&log));

        pool.register_module(ModelModule {
            module_id: "ml-core".to_owned(),
            kind: ModelKind::Ml,
            efficiency_score: 0.7,
            complexity_weight: 0.9,
            max_replicas: 3,
        });
        pool.register_module(ModelModule {
            module_id: "ai-core".to_owned(),
            kind: ModelKind::Ai,
            efficiency_score: 0.8,
            complexity_weight: 0.6,
            max_replicas: 2,
        });

        let demand = ReplicaDemand {
            dimension_id: DimensionId::new(),
            task_id: TaskId::new(),
            model_id: "ensemble-1".to_owned(),
            complexity: 0.85,
            efficiency_target: 0.5,
            max_replicas: 3,
            policy: ReplicaPolicy::default(),
        };

        let provisioning = pool.provision(demand).unwrap();
        assert!(provisioning.plan.total_replicas >= 2);
        assert_eq!(provisioning.replicas.len() as u32, provisioning.plan.total_replicas);
    }
}
