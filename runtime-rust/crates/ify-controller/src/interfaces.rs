//! Trait implementations: binds `ify-controller` concrete types to the
//! stable interfaces defined in `ify-interfaces`.
//!
//! Every `impl` block here is a **conformance claim** — it guarantees that
//! the concrete type satisfies the contract published in `ify-interfaces` and
//! documented in `docs/architecture/layer-interfaces.md`.
//!
//! ## API conformance tests
//!
//! The `#[cfg(test)]` section at the bottom of this module contains the
//! API conformance test suite required by Epic L.  Each test:
//! 1. Constructs the concrete type through the stable trait interface only.
//! 2. Exercises the full method surface.
//! 3. Asserts the expected postconditions.
//!
//! Run with `cargo test -p ify-controller interfaces`.

use ify_core::{ArtifactId, DimensionId, TaskId};
use ify_interfaces::{
    editor::{BlockId, EditorIntegrationApi, EditorRef, InterpreterRef, RuntimeHandle},
    event_bus::{EventBusApi, OrchestratorBusApi},
    mesh::{MeshArtifactApi, MeshSubscriberApi},
    node_execution::{NodeExecutorApi, NodePlannerApi, NodeReporterApi},
};
use tokio::sync::broadcast;
use tracing::debug;
use uuid::Uuid;

use crate::{
    action_log::{ActionLog, ActionLogEntry},
    graph::{FlowGraph, FlowGraphError, ValidationReport},
    mesh::{DiffPatch, MeshArtifact, MeshArtifactStore, MeshError, NodeSnapshot},
    orchestrator::{LocalOrchestrator, OrchestratorError, OrchestratorEvent},
    registry::{BlockRegistry, RegistryError},
};

// ---------------------------------------------------------------------------
// EventBusApi for ActionLog
// ---------------------------------------------------------------------------

impl EventBusApi for ActionLog {
    type Entry = ActionLogEntry;

    fn append(&self, entry: ActionLogEntry) {
        ActionLog::append(self, entry);
    }

    fn subscribe(&self) -> broadcast::Receiver<ActionLogEntry> {
        ActionLog::subscribe(self)
    }

    fn entries_for_dimension(&self, dim: DimensionId) -> Vec<ActionLogEntry> {
        ActionLog::entries_for_dimension(self, dim)
    }

    fn entries_for_task(&self, task_id: TaskId) -> Vec<ActionLogEntry> {
        ActionLog::entries_for_task(self, task_id)
    }

    fn all_entries(&self) -> Vec<ActionLogEntry> {
        ActionLog::all_entries(self)
    }

    fn len(&self) -> usize {
        ActionLog::len(self)
    }
}

// ---------------------------------------------------------------------------
// OrchestratorBusApi for LocalOrchestrator
// ---------------------------------------------------------------------------

impl OrchestratorBusApi for LocalOrchestrator {
    type Event = OrchestratorEvent;
    type Error = OrchestratorError;

    fn submit(
        &self,
        task_id: TaskId,
        dimension_id: DimensionId,
        priority: u8,
        payload: serde_json::Value,
    ) -> Result<(), OrchestratorError> {
        // Log priority and payload via tracing so they are preserved for
        // audit/replay.  The concrete submit stores only task identity; a
        // future evolution of LocalOrchestrator can promote these fields into
        // OrchestratorEvent once the data model supports it.
        debug!(
            %task_id,
            %dimension_id,
            priority,
            %payload,
            "OrchestratorBusApi::submit"
        );
        LocalOrchestrator::submit(self, task_id, dimension_id)
    }

    fn progress(
        &self,
        task_id: TaskId,
        percent: u8,
        message: &str,
    ) -> Result<(), OrchestratorError> {
        LocalOrchestrator::progress(self, task_id, percent, message)
    }

    fn complete(&self, task_id: TaskId) -> Result<(), OrchestratorError> {
        LocalOrchestrator::complete(self, task_id)
    }

    fn fail(&self, task_id: TaskId, error: &str) -> Result<(), OrchestratorError> {
        LocalOrchestrator::fail(self, task_id, error)
    }

    fn cancel(&self, task_id: TaskId) -> Result<(), OrchestratorError> {
        LocalOrchestrator::cancel(self, task_id)
    }

    fn replay(
        &self,
        task_id: TaskId,
    ) -> Result<Vec<OrchestratorEvent>, OrchestratorError> {
        LocalOrchestrator::replay(self, task_id)
    }

    fn subscribe(&self) -> broadcast::Receiver<OrchestratorEvent> {
        LocalOrchestrator::subscribe(self)
    }
}

// ---------------------------------------------------------------------------
// NodeExecutorApi + NodeReporterApi for LocalOrchestrator
// ---------------------------------------------------------------------------

impl NodeExecutorApi for LocalOrchestrator {
    type Error = OrchestratorError;

    fn submit(
        &self,
        task_id: TaskId,
        dimension_id: DimensionId,
        priority: u8,
        payload: serde_json::Value,
    ) -> Result<(), OrchestratorError> {
        debug!(
            %task_id,
            %dimension_id,
            priority,
            %payload,
            "NodeExecutorApi::submit"
        );
        LocalOrchestrator::submit(self, task_id, dimension_id)
    }

    fn cancel(&self, task_id: TaskId) -> Result<(), OrchestratorError> {
        LocalOrchestrator::cancel(self, task_id)
    }
}

impl NodeReporterApi for LocalOrchestrator {
    type Error = OrchestratorError;

    fn progress(
        &self,
        task_id: TaskId,
        percent: u8,
        message: &str,
    ) -> Result<(), OrchestratorError> {
        LocalOrchestrator::progress(self, task_id, percent, message)
    }

    fn complete(&self, task_id: TaskId) -> Result<(), OrchestratorError> {
        LocalOrchestrator::complete(self, task_id)
    }

    fn fail(
        &self,
        task_id: TaskId,
        error_message: &str,
    ) -> Result<(), OrchestratorError> {
        LocalOrchestrator::fail(self, task_id, error_message)
    }

    fn cancel(&self, task_id: TaskId) -> Result<(), OrchestratorError> {
        LocalOrchestrator::cancel(self, task_id)
    }
}

// ---------------------------------------------------------------------------
// MeshArtifactApi + MeshSubscriberApi for MeshArtifactStore
// ---------------------------------------------------------------------------

impl MeshArtifactApi for MeshArtifactStore {
    type Artifact = MeshArtifact;
    type Snapshot = NodeSnapshot;
    type Patch = DiffPatch;
    type Error = MeshError;

    fn produce(&self, artifact: MeshArtifact) -> ArtifactId {
        MeshArtifactStore::produce(self, artifact)
    }

    fn consume(&self, id: ArtifactId) -> Result<MeshArtifact, MeshError> {
        MeshArtifactStore::consume(self, id)
    }

    fn snapshot_node(
        &self,
        dimension_id: DimensionId,
        task_id: TaskId,
        node_id: Uuid,
        content: serde_json::Value,
    ) -> ArtifactId {
        // Concrete signature: (node_id, state, task_id, dimension_id)
        MeshArtifactStore::snapshot_node(self, node_id, content, task_id, dimension_id)
    }

    fn get_snapshot(&self, id: ArtifactId) -> Result<NodeSnapshot, MeshError> {
        MeshArtifactStore::get_snapshot(self, id)
    }

    fn patch(
        &self,
        dimension_id: DimensionId,
        task_id: TaskId,
        _node_id: Uuid,
        ops: serde_json::Value,
    ) -> ArtifactId {
        // Deserialize the JSON ops array into the concrete Vec<PatchOp>.
        // On parse failure, fall back to an empty ops list (the raw `ops` JSON
        // value is still preserved as the `after` field for audit purposes).
        let (before, after, patch_ops) =
            match serde_json::from_value::<Vec<crate::mesh::PatchOp>>(ops.clone()) {
                Ok(deserialized) => (serde_json::Value::Null, ops, deserialized),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "MeshArtifactApi::patch: failed to deserialize ops JSON; \
                         storing empty ops list for audit"
                    );
                    (serde_json::Value::Null, ops, vec![])
                }
            };
        // Concrete signature: (before, after, ops, task_id, dimension_id)
        MeshArtifactStore::patch(self, before, after, patch_ops, task_id, dimension_id)
    }

    fn get_patch(&self, id: ArtifactId) -> Result<DiffPatch, MeshError> {
        MeshArtifactStore::get_patch(self, id)
    }

    fn artifact_count(&self) -> usize {
        MeshArtifactStore::artifact_count(self)
    }
}

impl MeshSubscriberApi for MeshArtifactStore {
    fn subscribe(&self) -> broadcast::Receiver<ArtifactId> {
        MeshArtifactStore::subscribe(self)
    }
}

// ---------------------------------------------------------------------------
// EditorIntegrationApi for BlockRegistry
// ---------------------------------------------------------------------------

impl EditorIntegrationApi for BlockRegistry {
    type Error = RegistryError;

    fn register_block(&self, dimension_id: DimensionId, task_id: TaskId) -> BlockId {
        BlockRegistry::register_block(self, dimension_id, task_id)
    }

    fn create_editor(
        &self,
        block_id: BlockId,
        language: &str,
    ) -> Result<EditorRef, RegistryError> {
        // Concrete method returns the editor UUID; look up the full record after.
        BlockRegistry::create_editor(self, block_id, language)?;
        let ei =
            BlockRegistry::editor_for(self, block_id).ok_or(RegistryError::InternalInvariant {
                id: block_id,
                reason: "editor not found immediately after create_editor succeeded",
            })?;
        Ok(EditorRef {
            id: ei.id,
            dimension_id: ei.dimension_id,
            language: ei.language,
        })
    }

    fn attach_interpreter(
        &self,
        block_id: BlockId,
        interpreter_type: &str,
        config: serde_json::Value,
    ) -> Result<InterpreterRef, RegistryError> {
        // Concrete method returns (); look up the editor id from the stored record.
        BlockRegistry::attach_interpreter(self, block_id, interpreter_type, config)?;
        let editor_id =
            BlockRegistry::editor_for(self, block_id)
                .ok_or(RegistryError::InternalInvariant {
                    id: block_id,
                    reason: "editor not found after attach_interpreter succeeded",
                })?
                .id;
        Ok(InterpreterRef {
            id: editor_id,
            interpreter_type: interpreter_type.to_owned(),
        })
    }

    fn bind_runtime(&self, block_id: BlockId) -> Result<RuntimeHandle, RegistryError> {
        let rb = BlockRegistry::bind_runtime(self, block_id)?;
        Ok(RuntimeHandle {
            id: rb.block_id,
            task_id: rb.task_id,
            executor_endpoint: format!("local://{}", rb.block_id),
        })
    }

    fn editor_for(&self, block_id: BlockId) -> Option<EditorRef> {
        BlockRegistry::editor_for(self, block_id).map(|ei| EditorRef {
            id: ei.id,
            dimension_id: ei.dimension_id,
            language: ei.language,
        })
    }

    fn binding_for(&self, block_id: BlockId) -> Option<RuntimeHandle> {
        BlockRegistry::binding_for(self, block_id).map(|rb| RuntimeHandle {
            id: rb.block_id,
            task_id: rb.task_id,
            executor_endpoint: format!("local://{}", rb.block_id),
        })
    }
}

// ---------------------------------------------------------------------------
// NodePlannerApi for FlowGraph
// ---------------------------------------------------------------------------

impl NodePlannerApi for FlowGraph {
    /// The execution plan is a topologically sorted list of node IDs.
    type Plan = Vec<Uuid>;
    type Error = FlowGraphError;

    /// Produce a topologically ordered execution plan.
    ///
    /// `dimension_id` is validated against the graph's own dimension; a
    /// [`FlowGraphError::NodeNotFound`] is returned on mismatch (using the
    /// sentinel `Uuid::nil()`) to keep the error type minimal.
    ///
    /// Returns [`Err`] if the graph contains a cycle.
    fn plan(&self, dimension_id: DimensionId) -> Result<Vec<Uuid>, FlowGraphError> {
        if self.schema.dimension_id != dimension_id {
            return Err(FlowGraphError::DimensionMismatch {
                expected: self.schema.dimension_id,
                got: dimension_id,
            });
        }
        self.topological_order()
    }

    /// Validate the graph and return all issues as human-readable strings.
    ///
    /// Returns `Ok(())` when the graph is valid; `Err(issues)` otherwise.
    fn validate(&self, dimension_id: DimensionId) -> Result<(), Vec<String>> {
        if self.schema.dimension_id != dimension_id {
            return Err(vec![format!(
                "dimension mismatch: graph owns {}, caller requested {}",
                self.schema.dimension_id,
                dimension_id,
            )]);
        }
        let report: ValidationReport = FlowGraph::validate(self);
        if report.valid {
            Ok(())
        } else {
            Err(report
                .issues
                .into_iter()
                .map(|issue| format!("[{}] {}", issue.kind, issue.description))
                .collect())
        }
    }
}

// ---------------------------------------------------------------------------
// API conformance tests (Epic L requirement)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod conformance_tests {
    //! Conformance tests verify that each `ify-controller` implementation
    //! satisfies the stable `ify-interfaces` trait contracts.

    use std::sync::Arc;

    use super::*;
    use crate::{
        action_log::{Actor, EventType},
        graph::{FlowGraph, FlowGraphError},
        mesh::MeshArtifactBuilder,
        task_allocator::TaskAllocator,
    };
    use ify_core::{DimensionId, TaskId};
    use ify_interfaces::{
        editor::EditorIntegrationApi,
        event_bus::{EventBusApi, OrchestratorBusApi},
        mesh::{MeshArtifactApi, MeshSubscriberApi},
        node_execution::{NodeExecutorApi, NodePlannerApi, NodeReporterApi},
    };

    // --- EventBusApi conformance ---

    #[test]
    fn event_bus_append_and_query() {
        let log: Arc<ActionLog> = ActionLog::new(32);
        let dim = DimensionId::new();
        let task = TaskId::new();
        let entry = ActionLogEntry::new(
            EventType::TaskSubmitted,
            Actor::System,
            Some(dim),
            Some(task),
            serde_json::json!({"priority": 5, "queue_depth": 0}),
        );

        // Use the trait methods exclusively.
        let bus: &dyn EventBusApi<Entry = ActionLogEntry> = log.as_ref();
        assert!(bus.is_empty());
        bus.append(entry.clone());
        assert_eq!(bus.len(), 1);
        assert!(!bus.is_empty());
        assert_eq!(bus.entries_for_dimension(dim).len(), 1);
        assert_eq!(bus.entries_for_task(task).len(), 1);
        assert_eq!(bus.all_entries().len(), 1);
    }

    #[tokio::test]
    async fn event_bus_subscribe_receives_entry() {
        let log: Arc<ActionLog> = ActionLog::new(16);
        let bus: &dyn EventBusApi<Entry = ActionLogEntry> = log.as_ref();
        let mut rx = bus.subscribe();

        let dim = DimensionId::new();
        bus.append(ActionLogEntry::new(
            EventType::TaskStarted,
            Actor::System,
            Some(dim),
            None,
            serde_json::json!({"worker_id": "w1"}),
        ));

        let received = rx.recv().await.expect("should receive entry");
        assert_eq!(received.event_type, EventType::TaskStarted);
    }

    // --- OrchestratorBusApi / NodeExecutorApi / NodeReporterApi conformance ---

    #[test]
    fn orchestrator_bus_submit_and_complete() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let orch = LocalOrchestrator::new(dim, Arc::clone(&log), 32);

        // Submit via OrchestratorBusApi (deref Arc to get &LocalOrchestrator)
        let bus: &dyn OrchestratorBusApi<Event = OrchestratorEvent, Error = OrchestratorError> =
            orch.as_ref();
        bus.submit(task_id, dim, 5, serde_json::json!({}))
            .expect("submit should succeed");
        bus.progress(task_id, 50, "halfway").expect("progress should succeed");
        bus.complete(task_id).expect("complete should succeed");

        let replayed = bus.replay(task_id).expect("replay should succeed");
        assert!(replayed.len() >= 2, "should have at least Submitted + Completed");
    }

    #[test]
    fn orchestrator_bus_fail_and_replay() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let orch = LocalOrchestrator::new(dim, Arc::clone(&log), 32);
        let bus: &dyn OrchestratorBusApi<Event = OrchestratorEvent, Error = OrchestratorError> =
            orch.as_ref();
        bus.submit(task_id, dim, 1, serde_json::json!({})).unwrap();
        bus.fail(task_id, "something broke").unwrap();

        let replayed = bus.replay(task_id).unwrap();
        assert!(replayed
            .iter()
            .any(|e| matches!(e, OrchestratorEvent::Failed { .. })));
    }

    #[test]
    fn node_executor_and_reporter_via_traits() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let orch = LocalOrchestrator::new(dim, Arc::clone(&log), 32);

        // Use NodeExecutorApi (deref Arc)
        let executor: &dyn NodeExecutorApi<Error = OrchestratorError> = orch.as_ref();
        executor
            .submit(task_id, dim, 5, serde_json::json!({}))
            .unwrap();

        // Use NodeReporterApi (deref Arc)
        let reporter: &dyn NodeReporterApi<Error = OrchestratorError> = orch.as_ref();
        reporter.progress(task_id, 25, "a quarter").unwrap();
        reporter.complete(task_id).unwrap();
    }

    #[test]
    fn node_reporter_cancel_via_trait() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let orch = LocalOrchestrator::new(dim, Arc::clone(&log), 32);
        let executor: &dyn NodeExecutorApi<Error = OrchestratorError> = orch.as_ref();
        executor
            .submit(task_id, dim, 1, serde_json::json!({}))
            .unwrap();

        let reporter: &dyn NodeReporterApi<Error = OrchestratorError> = orch.as_ref();
        reporter.cancel(task_id).unwrap();
    }

    // --- MeshArtifactApi / MeshSubscriberApi conformance ---

    #[test]
    fn mesh_produce_and_consume_via_trait() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let store = MeshArtifactStore::new(Arc::clone(&log), 32);
        let artifact = MeshArtifactBuilder::new(dim, task_id)
            .content_type("application/json")
            .payload(serde_json::json!({"value": 42}))
            .build();

        let mesh: &dyn MeshArtifactApi<
            Artifact = _,
            Snapshot = _,
            Patch = _,
            Error = MeshError,
        > = store.as_ref();

        let id = mesh.produce(artifact);
        assert_eq!(mesh.artifact_count(), 1);

        let recovered = mesh.consume(id).expect("should be able to consume");
        assert_eq!(recovered.payload, serde_json::json!({"value": 42}));

        // Double-consume should fail.
        assert!(mesh.consume(id).is_err());
    }

    #[test]
    fn mesh_snapshot_via_trait() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();
        let node_id = Uuid::new_v4();

        let store = MeshArtifactStore::new(Arc::clone(&log), 32);
        let mesh: &dyn MeshArtifactApi<
            Artifact = _,
            Snapshot = _,
            Patch = _,
            Error = MeshError,
        > = store.as_ref();

        let snap_id = mesh.snapshot_node(dim, task_id, node_id, serde_json::json!({"x": 1}));
        let snap = mesh.get_snapshot(snap_id).expect("snapshot should exist");
        assert_eq!(snap.node_id, node_id);
    }

    #[test]
    fn mesh_patch_via_trait() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();
        let node_id = Uuid::new_v4();

        let store = MeshArtifactStore::new(Arc::clone(&log), 32);
        let mesh: &dyn MeshArtifactApi<
            Artifact = _,
            Snapshot = _,
            Patch = _,
            Error = MeshError,
        > = store.as_ref();

        let patch_id = mesh.patch(
            dim,
            task_id,
            node_id,
            serde_json::json!([{"op": "replace", "path": "/x", "value": 2}]),
        );
        assert!(mesh.get_patch(patch_id).is_ok());
    }

    #[tokio::test]
    async fn mesh_subscriber_api() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let store = MeshArtifactStore::new(Arc::clone(&log), 32);
        let subscriber: &dyn MeshSubscriberApi = store.as_ref();
        let mut rx = subscriber.subscribe();

        let artifact = MeshArtifactBuilder::new(dim, task_id)
            .content_type("text/plain")
            .payload(serde_json::json!("hello"))
            .build();

        let mesh: &dyn MeshArtifactApi<
            Artifact = _,
            Snapshot = _,
            Patch = _,
            Error = MeshError,
        > = store.as_ref();
        let id = mesh.produce(artifact);

        let notified = rx.recv().await.expect("should be notified");
        assert_eq!(notified, id);
    }

    // --- EditorIntegrationApi conformance ---

    #[test]
    fn editor_integration_pipeline_via_trait() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let registry = BlockRegistry::new(Arc::clone(&log));
        let editor_api: &dyn EditorIntegrationApi<Error = RegistryError> = &registry;

        let block_id = editor_api.register_block(dim, task_id);

        let editor_ref = editor_api
            .create_editor(block_id, "rust")
            .expect("create_editor should succeed");
        assert_eq!(editor_ref.language, "rust");

        // editor_for should return something now.
        let looked_up = editor_api.editor_for(block_id);
        assert!(looked_up.is_some());
        assert_eq!(looked_up.unwrap().id, editor_ref.id);

        let interpreter_ref = editor_api
            .attach_interpreter(block_id, "lsp", serde_json::json!({}))
            .expect("attach_interpreter should succeed");
        assert_eq!(interpreter_ref.interpreter_type, "lsp");

        let runtime_handle = editor_api
            .bind_runtime(block_id)
            .expect("bind_runtime should succeed");
        assert!(!runtime_handle.executor_endpoint.is_empty());

        // binding_for should return something now.
        let binding = editor_api.binding_for(block_id);
        assert!(binding.is_some());
        assert_eq!(binding.unwrap().task_id, runtime_handle.task_id);
    }

    #[test]
    fn editor_integration_duplicate_editor_fails() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let alloc = TaskAllocator::new();
        alloc.register_dimension(dim);
        let task_id = alloc.next(dim).unwrap();

        let registry = BlockRegistry::new(Arc::clone(&log));
        let editor_api: &dyn EditorIntegrationApi<Error = RegistryError> = &registry;

        let block_id = editor_api.register_block(dim, task_id);
        editor_api.create_editor(block_id, "python").unwrap();
        assert!(
            editor_api.create_editor(block_id, "python").is_err(),
            "second create_editor should fail"
        );
    }

    // --- NodePlannerApi conformance ---

    #[test]
    fn node_planner_plan_empty_graph() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let task = TaskId::new();
        let graph = FlowGraph::new(dim, task, Arc::clone(&log));

        let planner: &dyn NodePlannerApi<Plan = Vec<uuid::Uuid>, Error = FlowGraphError> =
            &graph;
        let plan = planner.plan(dim).expect("empty graph should produce an empty plan");
        assert!(plan.is_empty(), "empty graph has no nodes to execute");
    }

    #[test]
    fn node_planner_validate_valid_graph() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let task = TaskId::new();
        let graph = FlowGraph::new(dim, task, Arc::clone(&log));

        let planner: &dyn NodePlannerApi<Plan = Vec<uuid::Uuid>, Error = FlowGraphError> =
            &graph;
        assert!(planner.validate(dim).is_ok(), "empty graph should be valid");
    }

    #[test]
    fn node_planner_dimension_mismatch_returns_error() {
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let other_dim = DimensionId::new();
        let task = TaskId::new();
        let graph = FlowGraph::new(dim, task, Arc::clone(&log));

        let planner: &dyn NodePlannerApi<Plan = Vec<uuid::Uuid>, Error = FlowGraphError> =
            &graph;
        assert!(
            planner.plan(other_dim).is_err(),
            "plan with wrong dimension should fail"
        );
        assert!(
            planner.validate(other_dim).is_err(),
            "validate with wrong dimension should fail"
        );
    }

    #[test]
    fn node_planner_plan_with_nodes() {
        use crate::graph::GraphNode;
        let log = ActionLog::new(32);
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mut graph = FlowGraph::new(dim, task, Arc::clone(&log));

        // Add two independent nodes.
        let node_a = GraphNode::new("node_a", "any");
        let node_b = GraphNode::new("node_b", "any");
        let _id_a = graph.add_node(node_a);
        let _id_b = graph.add_node(node_b);

        let planner: &dyn NodePlannerApi<Plan = Vec<uuid::Uuid>, Error = FlowGraphError> =
            &graph;
        let plan = planner.plan(dim).expect("plan should succeed for valid graph");
        assert_eq!(plan.len(), 2, "plan should contain both nodes");
    }

    // --- Semver version constants are accessible ---

    #[test]
    fn version_constants_accessible_from_controller() {
        use ify_interfaces::versioning::{
            EDITOR_INTEGRATION_API_VERSION, EVENT_BUS_API_VERSION, MESH_ARTIFACT_API_VERSION,
            NODE_EXECUTION_API_VERSION,
        };
        assert_eq!(EVENT_BUS_API_VERSION.major, 1);
        assert_eq!(MESH_ARTIFACT_API_VERSION.major, 1);
        assert_eq!(NODE_EXECUTION_API_VERSION.major, 1);
        assert_eq!(EDITOR_INTEGRATION_API_VERSION.major, 1);
    }
}

