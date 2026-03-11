//! # ify-controller — blockControllerGenerator Regime
//!
//! This crate implements the full **EPIC B** feature set for infinityOS.
//! It provides the `blockControllerGenerator` regime: the layer that manages
//! dimensional block controllers, wires editors and interpreters into the
//! runtime, routes tasks through the orchestrator, and maintains the mesh
//! artifact write path and node graph.
//!
//! ## Module map
//!
//! | Module | Epic B item |
//! |--------|-------------|
//! | [`action_log`] | ActionLog capture for every controller action |
//! | [`task_allocator`] | Per-dimension monotonic TaskID allocator + deterministic derivation |
//! | [`controller`] | BlockController lifecycle (create → link → isolate → dispose) + dimension validation |
//! | [`registry`] | Block registration pipeline (editor → interpreter → runtime) |
//! | [`orchestrator`] | Orchestrator dispatch hooks (submit, subscribe, cancel, replay) |
//! | [`mesh`] | Mesh-artifact write path (produce, consume, snapshot, diff/patch) |
//! | [`node`] | Seamless node adder with undo/redo + node customizer with templates/presets |
//! | [`flow_control`] | Advanced flow control engine with ML-aware decisions |
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use std::collections::HashMap;
//! use std::sync::Arc;
//! use ify_controller::{
//!     action_log::ActionLog,
//!     controller::BlockController,
//!     task_allocator::TaskAllocator,
//!     registry::BlockRegistry,
//!     orchestrator::LocalOrchestrator,
//!     mesh::{MeshArtifactStore, MeshArtifactBuilder},
//!     node::{NodeGraph, NodeCustomizer, NodeTemplate},
//! };
//! use ify_core::{DimensionId, TaskId};
//!
//! // Shared ActionLog
//! let log = ActionLog::new(256);
//!
//! // Dimension + task allocation
//! let alloc = TaskAllocator::new();
//! let dim = DimensionId::new();
//! alloc.register_dimension(dim);
//! let task_id = alloc.next(dim).unwrap();
//!
//! // Block controller lifecycle
//! let ctrl = BlockController::create(dim, task_id, Arc::clone(&log));
//! ctrl.link(dim).unwrap();
//! ctrl.isolate().unwrap();
//! ctrl.dispose().unwrap();
//!
//! // Block registration pipeline
//! let registry = BlockRegistry::new(Arc::clone(&log));
//! let block_id = registry.register_block(dim, task_id);
//! let _editor_id = registry.create_editor(block_id, "rust").unwrap();
//! registry.attach_interpreter(block_id, "lsp", serde_json::json!({})).unwrap();
//! let _binding = registry.bind_runtime(block_id).unwrap();
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod action_log;
pub mod connectors;
pub mod controller;
pub mod flow_control;
pub mod graph;
pub mod graph_query;
pub mod mesh;
pub mod model_runtime;
pub mod node;
pub mod node_instance;
pub mod orchestrator;
pub mod registry;
pub mod scheduler;
pub mod task_allocator;

// Re-export the most commonly used types at the crate root for ergonomics.
pub use action_log::{ActionLog, ActionLogEntry, Actor, EventType};
pub use controller::{BlockController, BlockControllerError, ControllerState};
pub use flow_control::{
    // Canvas attachments
    CanvasAttachment,
    // Chat payloads + instructions
    ChatAdaptation,
    ChatAdapter,
    ChatInstructionSelection,
    ChatPayload,
    ChatPayloadBuilder,
    ChatRequest,
    ChatSkillModule,
    ChatTool,
    // Supporting types
    ComparisonOperator,
    // Flow graph + execution
    FlowAdvance,
    FlowCondition,
    FlowContext,
    FlowControlEngine,
    FlowControlError,
    FlowGraph,
    FlowState,
    FlowStep,
    FlowStepKind,
    FlowTerminalStatus,
    FlowTransition,
    FlowVisualization,
    InstructionDataset,
    InstructionEntry,
    InstructionSource,
    ModelEvaluator,
    QualityGate,
    SnippetAttachment,
    SnippetSandboxProfile,
    TerminalSession,
};
// Sorted alphabetically for maintainability.
pub use model_runtime::{
    HyperparameterAdjustment,
    HyperparameterPolicy,
    HyperparameterRule,
    Hyperparameters,
    InMemoryReplicaKernel,
    ModelEnsemblePlan,
    ModelKind,
    ModelModule,
    ModelModuleSelection,
    ModelOptimizationDecision,
    ModelPerformanceManager,
    ModelProfile,
    ModelReloadPolicy,
    ModelReloadRequest,
    ModelReplicaError,
    ModelReplicaPool,
    ModelRuntimeError,
    PerformanceSample,
    ReplicaDemand,
    ReplicaHandle,
    ReplicaId,
    ReplicaKernel,
    ReplicaKernelError,
    ReplicaPolicy,
    ReplicaProvisioningResult,
};
pub use mesh::{MeshArtifactBuilder, MeshArtifactStore};
pub use node::{Node, NodeCustomizer, NodeGraph, NodeTemplate};
pub use orchestrator::{LocalOrchestrator, OrchestratorEvent};
pub use registry::{BlockRegistry, RuntimeBinding};
pub use task_allocator::TaskAllocator;

// Epic F — Flow Graph and Node Connectivity
pub use connectors::{ConnectorKind, ConnectorParam, ConnectorRegistry, ConnectorTemplate};
pub use graph::{
    FlowGraph as FlowGraphRuntime,
    FlowGraphError,
    FlowGraphSchema,
    GRAPH_SCHEMA_VERSION,
    GraphNode,
    GraphPatch,
    GraphPatchOp,
    Group,
    Link,
    NodeExecutionContract,
    NodeExecutionState,
    NodeProvenance,
    NodeRelation,
    PortDataType,
    PortDef,
    PortDirection,
    RelationKind,
    Subgraph,
    ValidationIssue,
    ValidationReport,
};
// Epic F — Node Communication and Data Sampling
pub use graph_query::{
    NodeCommunicator,
    NodeCondition,
    NodeMessage,
    NodeSample,
    NodeSelector,
};
// Epic J — Job Scheduling and Task Lifecycle
pub use scheduler::{
    DimensionQuota,
    PreemptionPolicy,
    RetryPolicy,
    SchedulerError,
    TaskLease,
    TaskPriority,
    TaskRecord,
    TaskScheduler,
    TaskSpec,
    TaskState,
    TaskTemplate,
};
// Epic N — Node Instance Grouping
pub use node_instance::{
    InstanceConfig,
    InstanceError,
    InstanceRegistry,
    InstanceTemplate,
    LockPolicy,
    NodeInstance,
    ParamInheritance,
    ParamSchema,
    PublishReceipt,
    PublishRequest,
    TemplateProvenance,
};
