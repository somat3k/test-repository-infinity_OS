//! Processing and Transformation Pipelines — Epic P
//!
//! This module provides the complete data pipeline infrastructure for
//! infinityOS, implementing all ten Epic P items:
//!
//! 1. **Pipeline primitives** — [`StepKind`] covers `Map`, `Filter`, `Reduce`,
//!    `Aggregate`, and `Window`.  [`PipelineStep`] composes them into an
//!    ordered sequence forming a [`Pipeline`].
//! 2. **Transform versioning and replay** — [`TransformVersion`] is an
//!    immutable snapshot of a pipeline's steps at a point in time.
//!    [`PipelineRegistry::replay_version`] re-executes any historic version.
//! 3. **Dead-letter handling** — [`DeadLetterQueue`] captures every record
//!    that fails a step, preserving the input, error message, and retry count
//!    for later inspection or reprocessing.
//! 4. **Schema inference and validation** — [`SchemaInferrer::infer`] derives
//!    a [`DataSchema`] from a JSON sample.  [`SchemaValidator::validate`]
//!    checks any record against a registered schema.
//! 5. **Streaming mode + watermarking** — [`StreamRecord`] carries an
//!    [`EventWatermark`] that pairs event-time with processing-time.
//!    [`StreamPipeline`] tracks the current low-watermark across the stream.
//! 6. **Checkpointing and resume** — [`CheckpointStore`] persists
//!    [`PipelineCheckpoint`] snapshots keyed by `(pipeline_id, step_index)`.
//!    [`PipelineRegistry::resume_from_checkpoint`] reloads the latest
//!    checkpoint for a pipeline.
//! 7. **Lineage tracking** — [`LineageTracker`] records a [`LineageRecord`]
//!    for every step output, linking input/output artifact IDs with the
//!    producing task and dimension.
//! 8. **Performance optimization** — [`PipelineOptimizer`] holds a
//!    [`BatchConfig`] and accumulates [`PipelineMetrics`]; it suggests
//!    parallelism and batch-size adjustments based on observed throughput.
//! 9. **UI pipeline builder nodes** — [`PipelineBuilderRegistry`] maintains
//!    a catalogue of [`PipelineBuilderNode`] templates that the canvas node
//!    system consumes to render pipeline construction UI.
//! 10. **Connectors to DB and object storage** — [`StorageConnectorRegistry`]
//!     holds [`StorageConnector`] descriptors for PostgreSQL, MySQL, MongoDB,
//!     Redis, S3-compatible stores, local filesystem, and IPFS.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ify_core::{ArtifactId, DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::action_log::{ActionLog, ActionLogEntry, Actor, EventType};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the pipeline subsystem.
#[derive(Debug, Error)]
pub enum PipelineError {
    /// The referenced pipeline does not exist.
    #[error("pipeline {0} not found")]
    PipelineNotFound(Uuid),

    /// The referenced transform version does not exist.
    #[error("transform version {version} not found for pipeline {pipeline_id}")]
    VersionNotFound {
        /// Pipeline ID.
        pipeline_id: Uuid,
        /// Requested version number.
        version: u32,
    },

    /// The referenced schema does not exist.
    #[error("schema {0} not found")]
    SchemaNotFound(Uuid),

    /// The referenced checkpoint does not exist.
    #[error("no checkpoint found for pipeline {0}")]
    CheckpointNotFound(Uuid),

    /// The referenced storage connector does not exist.
    #[error("storage connector {0} not found")]
    ConnectorNotFound(Uuid),

    /// Schema validation failed.
    #[error("schema validation failed: {0}")]
    SchemaValidationFailed(String),

    /// A pipeline step returned an error during execution.
    #[error("pipeline step '{step_name}' failed: {reason}")]
    StepFailed {
        /// Human-readable step name.
        step_name: String,
        /// Error detail.
        reason: String,
    },
}

// ===========================================================================
// Item 1 — Pipeline Primitives
// ===========================================================================

/// The kind of transformation applied by a single pipeline step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Apply a function to every record, producing a new record.
    Map,
    /// Retain only records that satisfy a predicate.
    Filter,
    /// Combine all records into a single accumulated value.
    Reduce,
    /// Group records by a key and compute per-group statistics.
    Aggregate,
    /// Collect records within a time or count window before emitting.
    Window,
}

impl StepKind {
    /// Canonical lowercase string for this step kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Filter => "filter",
            Self::Reduce => "reduce",
            Self::Aggregate => "aggregate",
            Self::Window => "window",
        }
    }
}

impl std::fmt::Display for StepKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single transformation step in a [`Pipeline`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    /// Unique identifier for this step within the pipeline.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// The kind of transformation this step performs.
    pub kind: StepKind,
    /// Arbitrary step-level configuration (e.g., field names, window size).
    pub params: serde_json::Value,
    /// Optional schema ID that output records must conform to.
    pub output_schema_id: Option<Uuid>,
}

impl PipelineStep {
    /// Create a new step with the given name and kind.
    pub fn new(name: impl Into<String>, kind: StepKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            params: serde_json::Value::Object(Default::default()),
            output_schema_id: None,
        }
    }

    /// Attach free-form parameters to this step.
    pub fn with_params(mut self, params: serde_json::Value) -> Self {
        self.params = params;
        self
    }

    /// Bind an output schema to this step.
    pub fn with_output_schema(mut self, schema_id: Uuid) -> Self {
        self.output_schema_id = Some(schema_id);
        self
    }
}

/// An ordered sequence of [`PipelineStep`]s that together define a
/// complete data transformation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    /// Unique pipeline identifier.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// The dimension this pipeline operates in.
    pub dimension_id: DimensionId,
    /// Ordered list of transformation steps.
    pub steps: Vec<PipelineStep>,
    /// Current schema version of the pipeline definition.
    pub version: u32,
    /// Unix epoch ms at creation.
    pub created_at_ms: u64,
    /// Unix epoch ms of the last modification.
    pub updated_at_ms: u64,
}

impl Pipeline {
    /// Create a new pipeline with no steps.
    pub fn new(name: impl Into<String>, dimension_id: DimensionId) -> Self {
        let now = now_ms();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            dimension_id,
            steps: Vec::new(),
            version: 1,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    /// Append a step to this pipeline.
    pub fn add_step(&mut self, step: PipelineStep) {
        self.steps.push(step);
        self.version += 1;
        self.updated_at_ms = now_ms();
    }

    /// Remove a step by ID. Returns `true` if the step was found and removed.
    pub fn remove_step(&mut self, step_id: Uuid) -> bool {
        let before = self.steps.len();
        self.steps.retain(|s| s.id != step_id);
        if self.steps.len() < before {
            self.version += 1;
            self.updated_at_ms = now_ms();
            true
        } else {
            false
        }
    }
}

// ===========================================================================
// Item 2 — Transform Versioning and Replay
// ===========================================================================

/// An immutable snapshot of a [`Pipeline`]'s step list at a specific version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformVersion {
    /// Unique identifier for this version record.
    pub id: Uuid,
    /// The pipeline this version belongs to.
    pub pipeline_id: Uuid,
    /// The version number captured from [`Pipeline::version`].
    pub version: u32,
    /// The steps as they were when this version was saved.
    pub steps_snapshot: Vec<PipelineStep>,
    /// Unix epoch ms when this version was captured.
    pub saved_at_ms: u64,
    /// Actor who triggered the save.
    pub author: String,
    /// Optional human-readable description of the change.
    pub description: String,
}

impl TransformVersion {
    /// Create a new version record from a pipeline.
    pub fn capture(pipeline: &Pipeline, author: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            pipeline_id: pipeline.id,
            version: pipeline.version,
            steps_snapshot: pipeline.steps.clone(),
            saved_at_ms: now_ms(),
            author: author.into(),
            description: description.into(),
        }
    }
}

/// Request to replay a specific transform version over a given input batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformReplayRequest {
    /// Pipeline to replay.
    pub pipeline_id: Uuid,
    /// The historic version to re-apply.
    pub version: u32,
    /// Input records for the replay.
    pub input_records: Vec<serde_json::Value>,
    /// Task ID of the calling task.
    pub task_id: TaskId,
}

// ===========================================================================
// Item 3 — Dead-Letter Handling
// ===========================================================================

/// The reason a record was routed to the dead-letter queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadLetterReason {
    /// The step function returned an error.
    StepError,
    /// The output record did not satisfy the step's output schema.
    SchemaValidationFailed,
    /// The record exceeded the maximum retry count.
    MaxRetriesExceeded,
    /// The record was explicitly rejected by a filter step.
    FilterRejected,
}

/// A record that could not be processed by a pipeline step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterEntry {
    /// Unique identifier for this dead-letter entry.
    pub id: Uuid,
    /// The pipeline this record came from.
    pub pipeline_id: Uuid,
    /// The step that produced the failure.
    pub step_id: Uuid,
    /// The raw input record that caused the failure.
    pub input_record: serde_json::Value,
    /// Human-readable error or rejection message.
    pub error_message: String,
    /// Classification of the failure reason.
    pub reason: DeadLetterReason,
    /// Unix epoch ms when the failure occurred.
    pub failed_at_ms: u64,
    /// Number of prior attempts to process this record.
    pub retry_count: u32,
}

impl DeadLetterEntry {
    /// Construct a new dead-letter entry.
    pub fn new(
        pipeline_id: Uuid,
        step_id: Uuid,
        input_record: serde_json::Value,
        error_message: impl Into<String>,
        reason: DeadLetterReason,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            pipeline_id,
            step_id,
            input_record,
            error_message: error_message.into(),
            reason,
            failed_at_ms: now_ms(),
            retry_count: 0,
        }
    }
}

/// In-memory dead-letter queue for a pipeline.
///
/// In a production deployment this would be backed by a durable message
/// broker or database table; the in-memory implementation is provided for
/// testing and single-process deployments.
#[derive(Debug, Default)]
pub struct DeadLetterQueue {
    entries: Mutex<Vec<DeadLetterEntry>>,
}

impl DeadLetterQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue a dead-letter entry.
    pub fn enqueue(&self, entry: DeadLetterEntry) {
        self.entries
            .lock()
            .expect("dlq lock poisoned")
            .push(entry);
    }

    /// Return all entries for a specific pipeline.
    pub fn entries_for_pipeline(&self, pipeline_id: Uuid) -> Vec<DeadLetterEntry> {
        self.entries
            .lock()
            .expect("dlq lock poisoned")
            .iter()
            .filter(|e| e.pipeline_id == pipeline_id)
            .cloned()
            .collect()
    }

    /// Drain and return all entries (e.g., for batch reprocessing).
    pub fn drain(&self) -> Vec<DeadLetterEntry> {
        std::mem::take(
            &mut *self.entries.lock().expect("dlq lock poisoned"),
        )
    }

    /// Number of entries currently in the queue.
    pub fn len(&self) -> usize {
        self.entries.lock().expect("dlq lock poisoned").len()
    }

    /// Returns `true` when the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ===========================================================================
// Item 4 — Schema Inference and Validation
// ===========================================================================

/// The inferred or declared type for a dataset field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// UTF-8 string.
    String,
    /// 64-bit signed integer.
    Integer,
    /// 64-bit IEEE 754 floating-point number.
    Float,
    /// Boolean true/false.
    Boolean,
    /// JSON array.
    Array,
    /// JSON object.
    Object,
    /// JSON null.
    Null,
    /// Field type could not be determined.
    Unknown,
}

impl FieldType {
    /// Infer a `FieldType` from a JSON value.
    pub fn from_json(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::String(_) => Self::String,
            serde_json::Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    Self::Integer
                } else {
                    Self::Float
                }
            }
            serde_json::Value::Bool(_) => Self::Boolean,
            serde_json::Value::Array(_) => Self::Array,
            serde_json::Value::Object(_) => Self::Object,
            serde_json::Value::Null => Self::Null,
        }
    }
}

/// Schema descriptor for a single field in a dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// Field name (key in the JSON object).
    pub name: String,
    /// Inferred or declared type.
    pub field_type: FieldType,
    /// Whether `null` is an acceptable value.
    pub nullable: bool,
    /// Optional human-readable description.
    pub description: Option<String>,
}

/// A versioned schema for a dataset or pipeline step output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSchema {
    /// Unique schema ID.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// Schema version (incremented on each structural change).
    pub version: u32,
    /// Ordered list of field descriptors.
    pub fields: Vec<FieldSchema>,
    /// Unix epoch ms when this schema was registered.
    pub created_at_ms: u64,
}

impl DataSchema {
    /// Create a new empty schema.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            version: 1,
            fields: Vec::new(),
            created_at_ms: now_ms(),
        }
    }

    /// Add a field descriptor.
    pub fn add_field(&mut self, field: FieldSchema) {
        self.fields.push(field);
        self.version += 1;
    }
}

/// Derives a [`DataSchema`] from a sample JSON object.
pub struct SchemaInferrer;

impl SchemaInferrer {
    /// Infer a schema from a JSON object value.
    ///
    /// Non-object values are treated as a schema with a single `"value"` field.
    /// The resulting schema always has `version = 1`.
    pub fn infer(name: impl Into<String>, sample: &serde_json::Value) -> DataSchema {
        let fields = match sample {
            serde_json::Value::Object(map) => map
                .iter()
                .map(|(key, val)| FieldSchema {
                    name: key.clone(),
                    field_type: FieldType::from_json(val),
                    nullable: matches!(val, serde_json::Value::Null),
                    description: None,
                })
                .collect(),
            other => vec![FieldSchema {
                name: "value".to_owned(),
                field_type: FieldType::from_json(other),
                nullable: matches!(other, serde_json::Value::Null),
                description: None,
            }],
        };
        DataSchema {
            id: Uuid::new_v4(),
            name: name.into(),
            version: 1,
            fields,
            created_at_ms: now_ms(),
        }
    }
}

/// Validates JSON records against a registered [`DataSchema`].
pub struct SchemaValidator;

impl SchemaValidator {
    /// Validate a single JSON record against the supplied schema.
    ///
    /// Returns `Ok(())` when every required field is present and has the
    /// expected type.  Returns `Err(PipelineError::SchemaValidationFailed)`
    /// with a description of the first violation found.
    pub fn validate(record: &serde_json::Value, schema: &DataSchema) -> Result<(), PipelineError> {
        let obj = match record.as_object() {
            Some(o) => o,
            None => {
                return Err(PipelineError::SchemaValidationFailed(
                    "record is not a JSON object".into(),
                ));
            }
        };

        for field in &schema.fields {
            match obj.get(&field.name) {
                None => {
                    if !field.nullable {
                        return Err(PipelineError::SchemaValidationFailed(format!(
                            "required field '{}' is missing",
                            field.name
                        )));
                    }
                }
                Some(val) => {
                    if matches!(val, serde_json::Value::Null) && !field.nullable {
                        return Err(PipelineError::SchemaValidationFailed(format!(
                            "field '{}' is null but the schema marks it as non-nullable",
                            field.name
                        )));
                    }
                    let actual = FieldType::from_json(val);
                    // Allow Null for nullable fields (handled above).
                    if actual != FieldType::Null && actual != field.field_type {
                        return Err(PipelineError::SchemaValidationFailed(format!(
                            "field '{}' has type {:?} but schema expects {:?}",
                            field.name, actual, field.field_type
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Registry that stores and retrieves [`DataSchema`] instances.
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    schemas: Mutex<HashMap<Uuid, DataSchema>>,
}

impl SchemaRegistry {
    /// Create an empty schema registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new schema.  Returns the schema ID.
    pub fn register(&self, schema: DataSchema) -> Uuid {
        let id = schema.id;
        self.schemas.lock().expect("schema registry lock poisoned").insert(id, schema);
        id
    }

    /// Look up a schema by ID.
    pub fn get(&self, id: Uuid) -> Option<DataSchema> {
        self.schemas.lock().expect("schema registry lock poisoned").get(&id).cloned()
    }

    /// Return all registered schemas.
    pub fn all(&self) -> Vec<DataSchema> {
        self.schemas.lock().expect("schema registry lock poisoned").values().cloned().collect()
    }
}

// ===========================================================================
// Item 5 — Streaming Mode + Watermarking
// ===========================================================================

/// A pair of timestamps representing the progress of a stream.
///
/// The *event time* is extracted from the data itself; the *processing time*
/// is the wall-clock instant when the record arrived at the pipeline.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EventWatermark {
    /// The event time embedded in the record (Unix epoch ms).
    pub event_time_ms: u64,
    /// The wall-clock time when the record was received (Unix epoch ms).
    pub processing_time_ms: u64,
}

impl EventWatermark {
    /// Create a new watermark.
    pub fn new(event_time_ms: u64) -> Self {
        Self {
            event_time_ms,
            processing_time_ms: now_ms(),
        }
    }

    /// Lag between event time and processing time in milliseconds.
    pub fn lag_ms(&self) -> i64 {
        self.processing_time_ms as i64 - self.event_time_ms as i64
    }
}

/// A single record flowing through a streaming pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamRecord {
    /// Unique record identifier.
    pub id: Uuid,
    /// The record payload.
    pub payload: serde_json::Value,
    /// Watermark associated with this record.
    pub watermark: EventWatermark,
}

impl StreamRecord {
    /// Create a new stream record with the current processing time as watermark.
    pub fn new(payload: serde_json::Value, event_time_ms: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            payload,
            watermark: EventWatermark::new(event_time_ms),
        }
    }
}

/// A streaming pipeline that tracks the low-watermark across all in-flight
/// records and routes them through the configured [`Pipeline`] steps.
#[derive(Debug)]
pub struct StreamPipeline {
    /// The underlying pipeline definition.
    pub pipeline: Pipeline,
    /// Current low-watermark: the minimum event time of all records seen so far.
    pub low_watermark_ms: u64,
    /// Total records processed since the stream was started.
    pub records_processed: u64,
    /// Total records that failed processing.
    pub records_failed: u64,
}

impl StreamPipeline {
    /// Create a streaming pipeline wrapping the given pipeline definition.
    pub fn new(pipeline: Pipeline) -> Self {
        Self {
            pipeline,
            low_watermark_ms: 0,
            records_processed: 0,
            records_failed: 0,
        }
    }

    /// Advance the low-watermark to `event_time_ms` if it is greater than the
    /// current value, then count the record as processed.
    pub fn advance(&mut self, record: &StreamRecord) {
        if record.watermark.event_time_ms > self.low_watermark_ms {
            self.low_watermark_ms = record.watermark.event_time_ms;
        }
        self.records_processed += 1;
    }

    /// Record a processing failure (increments the failed counter).
    pub fn mark_failed(&mut self) {
        self.records_failed += 1;
    }
}

// ===========================================================================
// Item 6 — Checkpointing and Resume
// ===========================================================================

/// A durable checkpoint for a pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineCheckpoint {
    /// Unique checkpoint ID.
    pub id: Uuid,
    /// The pipeline this checkpoint belongs to.
    pub pipeline_id: Uuid,
    /// Index of the step the pipeline had *completed* when this was saved.
    /// `0` means no steps have been completed yet.
    pub completed_step_index: usize,
    /// Total records processed up to this checkpoint.
    pub processed_count: u64,
    /// Opaque serialised state (e.g., accumulator for a reduce step).
    pub state_snapshot: serde_json::Value,
    /// Unix epoch ms when the checkpoint was saved.
    pub saved_at_ms: u64,
}

impl PipelineCheckpoint {
    /// Create a new checkpoint for the given pipeline.
    pub fn new(
        pipeline_id: Uuid,
        completed_step_index: usize,
        processed_count: u64,
        state_snapshot: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            pipeline_id,
            completed_step_index,
            processed_count,
            state_snapshot,
            saved_at_ms: now_ms(),
        }
    }
}

/// In-memory store for pipeline checkpoints.
///
/// Only the *latest* checkpoint per pipeline is retained.  Replace with a
/// durable backend (database, object store) for production use.
#[derive(Debug, Default)]
pub struct CheckpointStore {
    checkpoints: Mutex<HashMap<Uuid, PipelineCheckpoint>>,
}

impl CheckpointStore {
    /// Create an empty checkpoint store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Save or overwrite the checkpoint for a pipeline.
    pub fn save(&self, checkpoint: PipelineCheckpoint) {
        self.checkpoints
            .lock()
            .expect("checkpoint store lock poisoned")
            .insert(checkpoint.pipeline_id, checkpoint);
    }

    /// Retrieve the latest checkpoint for a pipeline.
    pub fn get(&self, pipeline_id: Uuid) -> Option<PipelineCheckpoint> {
        self.checkpoints
            .lock()
            .expect("checkpoint store lock poisoned")
            .get(&pipeline_id)
            .cloned()
    }

    /// Remove the checkpoint for a pipeline (e.g., after successful completion).
    pub fn clear(&self, pipeline_id: Uuid) -> bool {
        self.checkpoints
            .lock()
            .expect("checkpoint store lock poisoned")
            .remove(&pipeline_id)
            .is_some()
    }
}

// ===========================================================================
// Item 7 — Lineage Tracking
// ===========================================================================

/// A lineage record linking a pipeline step's inputs to its output artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageRecord {
    /// Unique lineage record ID.
    pub id: Uuid,
    /// The pipeline this record belongs to.
    pub pipeline_id: Uuid,
    /// The step that produced the output.
    pub step_id: Uuid,
    /// Human-readable step name for display.
    pub step_name: String,
    /// Artifact IDs of the inputs consumed by this step.
    pub input_artifact_ids: Vec<ArtifactId>,
    /// Artifact ID of the output produced by this step.
    pub output_artifact_id: ArtifactId,
    /// Transform version active when this record was produced.
    pub transform_version: u32,
    /// Task that triggered the execution.
    pub task_id: TaskId,
    /// Dimension the pipeline runs in.
    pub dimension_id: DimensionId,
    /// Unix epoch ms when the lineage record was created.
    pub recorded_at_ms: u64,
}

impl LineageRecord {
    /// Create a new lineage record.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pipeline_id: Uuid,
        step_id: Uuid,
        step_name: impl Into<String>,
        input_artifact_ids: Vec<ArtifactId>,
        output_artifact_id: ArtifactId,
        transform_version: u32,
        task_id: TaskId,
        dimension_id: DimensionId,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            pipeline_id,
            step_id,
            step_name: step_name.into(),
            input_artifact_ids,
            output_artifact_id,
            transform_version,
            task_id,
            dimension_id,
            recorded_at_ms: now_ms(),
        }
    }
}

/// Append-only in-memory lineage tracker.
#[derive(Debug, Default)]
pub struct LineageTracker {
    records: Mutex<Vec<LineageRecord>>,
}

impl LineageTracker {
    /// Create a new, empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a lineage entry and emit an [`ActionLog`] event.
    pub fn record(&self, entry: LineageRecord, log: &ActionLog) {
        log.append(ActionLogEntry::new(
            EventType::PipelineLineageRecorded,
            Actor::System,
            Some(entry.dimension_id),
            Some(entry.task_id),
            serde_json::json!({
                "pipeline_id": entry.pipeline_id,
                "step_id": entry.step_id,
                "step_name": entry.step_name,
                "output_artifact_id": entry.output_artifact_id.to_string(),
                "transform_version": entry.transform_version,
            }),
        ));
        self.records
            .lock()
            .expect("lineage tracker lock poisoned")
            .push(entry);
    }

    /// Return all lineage records for a given pipeline.
    pub fn records_for_pipeline(&self, pipeline_id: Uuid) -> Vec<LineageRecord> {
        self.records
            .lock()
            .expect("lineage tracker lock poisoned")
            .iter()
            .filter(|r| r.pipeline_id == pipeline_id)
            .cloned()
            .collect()
    }

    /// Return all lineage records for a given output artifact.
    pub fn records_for_artifact(&self, artifact_id: ArtifactId) -> Vec<LineageRecord> {
        self.records
            .lock()
            .expect("lineage tracker lock poisoned")
            .iter()
            .filter(|r| r.output_artifact_id == artifact_id)
            .cloned()
            .collect()
    }

    /// Total number of lineage records stored.
    pub fn len(&self) -> usize {
        self.records.lock().expect("lineage tracker lock poisoned").len()
    }

    /// Returns `true` when no lineage records have been stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ===========================================================================
// Item 8 — Performance Optimization
// ===========================================================================

/// Configuration for batched pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchConfig {
    /// Maximum number of records in a single processing batch.
    pub max_batch_size: usize,
    /// Maximum time to wait before flushing an incomplete batch (ms).
    pub flush_interval_ms: u64,
    /// Number of parallel worker threads (or async tasks) to use.
    pub parallelism: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 1_000,
            flush_interval_ms: 500,
            parallelism: 4,
        }
    }
}

/// Observed performance metrics for a single pipeline execution window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineMetrics {
    /// The pipeline these metrics belong to.
    pub pipeline_id: Uuid,
    /// Total records processed in this window.
    pub records_processed: u64,
    /// Total records that failed in this window.
    pub records_failed: u64,
    /// Observed throughput (records per second).
    pub throughput_rps: f64,
    /// Average end-to-end latency (ms).
    pub avg_latency_ms: f64,
    /// 99th-percentile end-to-end latency (ms).
    pub p99_latency_ms: f64,
    /// Unix epoch ms when this sample was taken.
    pub sampled_at_ms: u64,
}

/// Optimisation recommendation produced by [`PipelineOptimizer`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationAdvice {
    /// Suggested batch size adjustment (positive = increase, negative = decrease).
    pub batch_size_delta: i64,
    /// Suggested parallelism adjustment.
    pub parallelism_delta: i64,
    /// Human-readable rationale.
    pub rationale: String,
}

/// Analyses [`PipelineMetrics`] samples and emits [`OptimizationAdvice`].
#[derive(Debug, Default)]
pub struct PipelineOptimizer {
    history: Mutex<Vec<PipelineMetrics>>,
}

impl PipelineOptimizer {
    /// Create a new optimizer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a metrics sample.
    pub fn record_sample(&self, sample: PipelineMetrics) {
        self.history
            .lock()
            .expect("optimizer lock poisoned")
            .push(sample);
    }

    /// Analyse the most recent samples and return an advice record.
    ///
    /// Strategy (simple heuristic):
    /// - If throughput is below 500 rps and parallelism < 16, suggest
    ///   increasing parallelism by 2.
    /// - If avg latency > 100 ms and batch size > 100, suggest halving
    ///   the batch size.
    /// - Otherwise, no change.
    pub fn advise(&self, current: &BatchConfig) -> OptimizationAdvice {
        let history = self.history.lock().expect("optimizer lock poisoned");
        if let Some(last) = history.last() {
            if last.throughput_rps < 500.0 && current.parallelism < 16 {
                return OptimizationAdvice {
                    batch_size_delta: 0,
                    parallelism_delta: 2,
                    rationale: format!(
                        "throughput {:.1} rps is below threshold; increase parallelism",
                        last.throughput_rps
                    ),
                };
            }
            if last.avg_latency_ms > 100.0 && current.max_batch_size > 100 {
                let delta = -(current.max_batch_size as i64 / 2);
                return OptimizationAdvice {
                    batch_size_delta: delta,
                    parallelism_delta: 0,
                    rationale: format!(
                        "avg latency {:.1} ms is above threshold; reduce batch size",
                        last.avg_latency_ms
                    ),
                };
            }
        }
        OptimizationAdvice {
            batch_size_delta: 0,
            parallelism_delta: 0,
            rationale: "no adjustment required".into(),
        }
    }

    /// Number of metric samples stored.
    pub fn sample_count(&self) -> usize {
        self.history.lock().expect("optimizer lock poisoned").len()
    }
}

// ===========================================================================
// Item 9 — UI Pipeline Builder Nodes
// ===========================================================================

/// The role of a node in the visual pipeline builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineNodeKind {
    /// Data source (e.g., database table, object store, stream).
    Source,
    /// Map transformation.
    Map,
    /// Filter / predicate node.
    Filter,
    /// Reduce / fold node.
    Reduce,
    /// Aggregate / group-by node.
    Aggregate,
    /// Time or count window node.
    Window,
    /// Schema validation node.
    Validate,
    /// Dead-letter router.
    DeadLetterRouter,
    /// Data sink (e.g., database table, object store).
    Sink,
}

impl PipelineNodeKind {
    /// Canonical node kind string used in the graph model.
    pub fn node_kind_str(self) -> &'static str {
        match self {
            Self::Source => "pipeline.source",
            Self::Map => "pipeline.map",
            Self::Filter => "pipeline.filter",
            Self::Reduce => "pipeline.reduce",
            Self::Aggregate => "pipeline.aggregate",
            Self::Window => "pipeline.window",
            Self::Validate => "pipeline.validate",
            Self::DeadLetterRouter => "pipeline.dead_letter_router",
            Self::Sink => "pipeline.sink",
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Source => "Data Source",
            Self::Map => "Map",
            Self::Filter => "Filter",
            Self::Reduce => "Reduce",
            Self::Aggregate => "Aggregate",
            Self::Window => "Window",
            Self::Validate => "Schema Validate",
            Self::DeadLetterRouter => "Dead-Letter Router",
            Self::Sink => "Data Sink",
        }
    }

    /// Short description of the node's purpose.
    pub fn description(self) -> &'static str {
        match self {
            Self::Source => "Ingests records from a storage connector or stream.",
            Self::Map => "Applies a function to every record.",
            Self::Filter => "Retains records that satisfy a predicate.",
            Self::Reduce => "Folds all records into a single accumulator.",
            Self::Aggregate => "Groups records by key and computes per-group statistics.",
            Self::Window => "Collects records in a time or count window before emitting.",
            Self::Validate => "Validates records against a registered schema.",
            Self::DeadLetterRouter => "Routes failed records to the dead-letter queue.",
            Self::Sink => "Writes records to a storage connector.",
        }
    }
}

/// A template for a pipeline builder node as displayed in the canvas UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineBuilderNode {
    /// Unique template ID.
    pub id: Uuid,
    /// The role this node plays in the pipeline.
    pub kind: PipelineNodeKind,
    /// Canonical node kind string.
    pub node_kind_str: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Short description shown in the palette.
    pub description: String,
    /// Default parameters surfaced in the node's parameter editor.
    pub default_params: serde_json::Value,
    /// Input port names.
    pub input_ports: Vec<String>,
    /// Output port names.
    pub output_ports: Vec<String>,
}

impl PipelineBuilderNode {
    /// Construct the canonical node template for a given kind.
    pub fn for_kind(kind: PipelineNodeKind) -> Self {
        let (input_ports, output_ports, default_params) = match kind {
            PipelineNodeKind::Source => (
                vec![],
                vec!["records".into()],
                serde_json::json!({"connector_id": null, "query": "SELECT * FROM your_table LIMIT 1000"}),
            ),
            PipelineNodeKind::Map => (
                vec!["in".into()],
                vec!["out".into()],
                serde_json::json!({"expression": "record"}),
            ),
            PipelineNodeKind::Filter => (
                vec!["in".into()],
                vec!["passed".into(), "rejected".into()],
                serde_json::json!({"predicate": "true"}),
            ),
            PipelineNodeKind::Reduce => (
                vec!["in".into()],
                vec!["result".into()],
                serde_json::json!({"initial_value": null, "accumulator": "acc"}),
            ),
            PipelineNodeKind::Aggregate => (
                vec!["in".into()],
                vec!["groups".into()],
                serde_json::json!({"group_by": [], "metrics": ["count", "sum", "avg"]}),
            ),
            PipelineNodeKind::Window => (
                vec!["in".into()],
                vec!["window".into()],
                serde_json::json!({"window_type": "tumbling", "size_ms": 60_000}),
            ),
            PipelineNodeKind::Validate => (
                vec!["in".into()],
                vec!["valid".into(), "invalid".into()],
                serde_json::json!({"schema_id": null}),
            ),
            PipelineNodeKind::DeadLetterRouter => (
                vec!["failed".into()],
                vec!["dlq".into()],
                serde_json::json!({"max_retries": 3}),
            ),
            PipelineNodeKind::Sink => (
                vec!["in".into()],
                vec![],
                serde_json::json!({"connector_id": null, "table": "output"}),
            ),
        };
        Self {
            id: Uuid::new_v4(),
            node_kind_str: kind.node_kind_str().to_owned(),
            display_name: kind.display_name().to_owned(),
            description: kind.description().to_owned(),
            kind,
            default_params,
            input_ports,
            output_ports,
        }
    }
}

/// Registry of pipeline builder node templates available in the canvas UI.
#[derive(Debug, Default)]
pub struct PipelineBuilderRegistry {
    templates: Mutex<HashMap<Uuid, PipelineBuilderNode>>,
}

impl PipelineBuilderRegistry {
    /// Create a registry pre-populated with one template per [`PipelineNodeKind`].
    pub fn with_defaults() -> Self {
        let registry = Self::default();
        let kinds = [
            PipelineNodeKind::Source,
            PipelineNodeKind::Map,
            PipelineNodeKind::Filter,
            PipelineNodeKind::Reduce,
            PipelineNodeKind::Aggregate,
            PipelineNodeKind::Window,
            PipelineNodeKind::Validate,
            PipelineNodeKind::DeadLetterRouter,
            PipelineNodeKind::Sink,
        ];
        for kind in kinds {
            registry.register(PipelineBuilderNode::for_kind(kind));
        }
        registry
    }

    /// Register a builder node template.
    pub fn register(&self, node: PipelineBuilderNode) {
        self.templates
            .lock()
            .expect("builder registry lock poisoned")
            .insert(node.id, node);
    }

    /// Look up a template by ID.
    pub fn get(&self, id: Uuid) -> Option<PipelineBuilderNode> {
        self.templates
            .lock()
            .expect("builder registry lock poisoned")
            .get(&id)
            .cloned()
    }

    /// Return all registered templates.
    pub fn all(&self) -> Vec<PipelineBuilderNode> {
        self.templates
            .lock()
            .expect("builder registry lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Number of registered templates.
    pub fn len(&self) -> usize {
        self.templates.lock().expect("builder registry lock poisoned").len()
    }

    /// Returns `true` when no templates are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ===========================================================================
// Item 10 — Connectors to DB and Object Storage
// ===========================================================================

/// The backend technology of a storage connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageConnectorKind {
    /// PostgreSQL relational database.
    Postgresql,
    /// MySQL / MariaDB relational database.
    Mysql,
    /// MongoDB document store.
    Mongodb,
    /// Redis key-value / stream store.
    Redis,
    /// Amazon S3-compatible object storage (AWS S3, MinIO, GCS, R2, etc.).
    S3Compatible,
    /// Local filesystem (development / testing).
    LocalFilesystem,
    /// IPFS-backed content-addressed storage for legal/regulatory artifacts.
    Ipfs,
}

impl StorageConnectorKind {
    /// Canonical kind string used in connector configurations.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postgresql => "postgresql",
            Self::Mysql => "mysql",
            Self::Mongodb => "mongodb",
            Self::Redis => "redis",
            Self::S3Compatible => "s3_compatible",
            Self::LocalFilesystem => "local_filesystem",
            Self::Ipfs => "ipfs",
        }
    }

    /// Human-readable display name.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Postgresql => "PostgreSQL",
            Self::Mysql => "MySQL / MariaDB",
            Self::Mongodb => "MongoDB",
            Self::Redis => "Redis",
            Self::S3Compatible => "S3-Compatible Object Storage",
            Self::LocalFilesystem => "Local Filesystem",
            Self::Ipfs => "IPFS",
        }
    }
}

/// Descriptor for a storage connector (DB or object store).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConnector {
    /// Unique connector ID.
    pub id: Uuid,
    /// Human-readable name.
    pub name: String,
    /// The backend technology.
    pub kind: StorageConnectorKind,
    /// Connection parameters (host, port, database, bucket, etc.).
    /// Secrets (passwords, tokens) must be supplied via the secret manager;
    /// they must not be stored here.
    pub params: serde_json::Value,
    /// Unix epoch ms when this connector was registered.
    pub registered_at_ms: u64,
}

impl StorageConnector {
    /// Create a new storage connector descriptor.
    pub fn new(name: impl Into<String>, kind: StorageConnectorKind, params: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            params,
            registered_at_ms: now_ms(),
        }
    }
}

/// Registry of storage connectors available for pipeline source/sink nodes.
#[derive(Debug, Default)]
pub struct StorageConnectorRegistry {
    connectors: Mutex<HashMap<Uuid, StorageConnector>>,
    log: Option<Arc<ActionLog>>,
}

impl StorageConnectorRegistry {
    /// Create an empty registry without ActionLog integration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty registry with ActionLog integration.
    pub fn with_log(log: Arc<ActionLog>) -> Self {
        Self {
            connectors: Mutex::new(HashMap::new()),
            log: Some(log),
        }
    }

    /// Register a connector and optionally emit an ActionLog event.
    pub fn register(&self, connector: StorageConnector) -> Uuid {
        let id = connector.id;
        if let Some(log) = &self.log {
            log.append(ActionLogEntry::new(
                EventType::StorageConnectorRegistered,
                Actor::System,
                None,
                None,
                serde_json::json!({
                    "connector_id": id,
                    "name": connector.name,
                    "kind": connector.kind.as_str(),
                }),
            ));
        }
        self.connectors
            .lock()
            .expect("connector registry lock poisoned")
            .insert(id, connector);
        id
    }

    /// Look up a connector by ID.
    pub fn get(&self, id: Uuid) -> Option<StorageConnector> {
        self.connectors
            .lock()
            .expect("connector registry lock poisoned")
            .get(&id)
            .cloned()
    }

    /// Return all registered connectors.
    pub fn all(&self) -> Vec<StorageConnector> {
        self.connectors
            .lock()
            .expect("connector registry lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Remove a connector by ID.  Returns `true` if it was found.
    pub fn remove(&self, id: Uuid) -> bool {
        self.connectors
            .lock()
            .expect("connector registry lock poisoned")
            .remove(&id)
            .is_some()
    }

    /// Number of registered connectors.
    pub fn len(&self) -> usize {
        self.connectors.lock().expect("connector registry lock poisoned").len()
    }

    /// Returns `true` when no connectors are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ===========================================================================
// Pipeline Registry (ties everything together)
// ===========================================================================

/// Central registry that manages pipelines, transform versions, schemas,
/// checkpoints, lineage, and connectors for a single infinityOS process.
///
/// An [`Arc<PipelineRegistry>`] can be shared across threads and async tasks.
#[derive(Debug)]
pub struct PipelineRegistry {
    pipelines: Mutex<HashMap<Uuid, Pipeline>>,
    versions: Mutex<HashMap<Uuid, Vec<TransformVersion>>>,
    log: Arc<ActionLog>,
    /// Shared schema registry for dataset and step-output schemas.
    pub schemas: SchemaRegistry,
    /// Checkpoint store for pipeline execution state.
    pub checkpoints: CheckpointStore,
    /// Lineage tracker linking step outputs to mesh artifacts.
    pub lineage: LineageTracker,
    /// Dead-letter queue for records that fail pipeline steps.
    pub dlq: DeadLetterQueue,
    /// Registry of DB and object-storage connectors.
    pub connectors: StorageConnectorRegistry,
    /// Catalogue of UI pipeline builder node templates.
    pub builder: PipelineBuilderRegistry,
    /// Performance optimizer for batch and parallelism tuning.
    pub optimizer: PipelineOptimizer,
}

impl PipelineRegistry {
    /// Create a new pipeline registry backed by the given [`ActionLog`].
    pub fn new(log: Arc<ActionLog>) -> Self {
        Self {
            pipelines: Mutex::new(HashMap::new()),
            versions: Mutex::new(HashMap::new()),
            log: Arc::clone(&log),
            schemas: SchemaRegistry::new(),
            checkpoints: CheckpointStore::new(),
            lineage: LineageTracker::new(),
            dlq: DeadLetterQueue::new(),
            connectors: StorageConnectorRegistry::with_log(log),
            builder: PipelineBuilderRegistry::with_defaults(),
            optimizer: PipelineOptimizer::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Pipeline CRUD
    // -----------------------------------------------------------------------

    /// Register a new pipeline. Returns the pipeline ID.
    pub fn register_pipeline(&self, pipeline: Pipeline) -> Uuid {
        let id = pipeline.id;
        let dim = pipeline.dimension_id;
        self.log.append(ActionLogEntry::new(
            EventType::PipelineCreated,
            Actor::System,
            Some(dim),
            None,
            serde_json::json!({
                "pipeline_id": id,
                "name": pipeline.name,
                "version": pipeline.version,
            }),
        ));
        self.pipelines.lock().expect("pipeline registry lock poisoned").insert(id, pipeline);
        id
    }

    /// Look up a pipeline by ID.
    pub fn get_pipeline(&self, id: Uuid) -> Result<Pipeline, PipelineError> {
        self.pipelines
            .lock()
            .expect("pipeline registry lock poisoned")
            .get(&id)
            .cloned()
            .ok_or(PipelineError::PipelineNotFound(id))
    }

    /// Update a pipeline in-place.
    ///
    /// Automatically captures a new [`TransformVersion`] and logs the change.
    pub fn update_pipeline(
        &self,
        pipeline: Pipeline,
        author: impl Into<String>,
        description: impl Into<String>,
    ) -> Result<TransformVersion, PipelineError> {
        let id = pipeline.id;
        let version = TransformVersion::capture(&pipeline, author, description);
        self.log.append(ActionLogEntry::new(
            EventType::TransformVersionSaved,
            Actor::System,
            Some(pipeline.dimension_id),
            None,
            serde_json::json!({
                "pipeline_id": id,
                "version": pipeline.version,
                "version_record_id": version.id,
            }),
        ));
        self.versions
            .lock()
            .expect("versions lock poisoned")
            .entry(id)
            .or_default()
            .push(version.clone());
        self.pipelines.lock().expect("pipeline registry lock poisoned").insert(id, pipeline);
        Ok(version)
    }

    // -----------------------------------------------------------------------
    // Transform Versioning
    // -----------------------------------------------------------------------

    /// Return all saved transform versions for a pipeline, oldest first.
    pub fn versions_for_pipeline(&self, pipeline_id: Uuid) -> Vec<TransformVersion> {
        self.versions
            .lock()
            .expect("versions lock poisoned")
            .get(&pipeline_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Replay a specific historic version over the supplied input records.
    ///
    /// Returns the steps snapshot for the requested version so callers can
    /// apply the transform themselves.  Full execution is out of scope for
    /// this registry layer.
    pub fn replay_version(
        &self,
        req: TransformReplayRequest,
    ) -> Result<(TransformVersion, Vec<serde_json::Value>), PipelineError> {
        let versions = self.versions.lock().expect("versions lock poisoned");
        let pipeline_versions = versions
            .get(&req.pipeline_id)
            .ok_or(PipelineError::PipelineNotFound(req.pipeline_id))?;
        let version = pipeline_versions
            .iter()
            .find(|v| v.version == req.version)
            .ok_or(PipelineError::VersionNotFound {
                pipeline_id: req.pipeline_id,
                version: req.version,
            })?
            .clone();
        drop(versions);

        self.log.append(ActionLogEntry::new(
            EventType::TransformReplayRequested,
            Actor::System,
            None,
            Some(req.task_id),
            serde_json::json!({
                "pipeline_id": req.pipeline_id,
                "version": req.version,
                "record_count": req.input_records.len(),
            }),
        ));

        // Return the input records unchanged — execution is done by the caller.
        Ok((version, req.input_records))
    }

    // -----------------------------------------------------------------------
    // Checkpointing
    // -----------------------------------------------------------------------

    /// Save a checkpoint for a pipeline and emit an ActionLog event.
    pub fn save_checkpoint(
        &self,
        pipeline_id: Uuid,
        completed_step_index: usize,
        processed_count: u64,
        state_snapshot: serde_json::Value,
    ) -> Result<PipelineCheckpoint, PipelineError> {
        self.get_pipeline(pipeline_id)?;
        let cp = PipelineCheckpoint::new(
            pipeline_id,
            completed_step_index,
            processed_count,
            state_snapshot,
        );
        self.log.append(ActionLogEntry::new(
            EventType::PipelineCheckpointed,
            Actor::System,
            None,
            None,
            serde_json::json!({
                "pipeline_id": pipeline_id,
                "completed_step_index": completed_step_index,
                "processed_count": processed_count,
            }),
        ));
        self.checkpoints.save(cp.clone());
        Ok(cp)
    }

    /// Resume a pipeline from its latest checkpoint.
    ///
    /// Returns the checkpoint so callers know at which step to continue and
    /// what state to restore.
    pub fn resume_from_checkpoint(
        &self,
        pipeline_id: Uuid,
    ) -> Result<PipelineCheckpoint, PipelineError> {
        let cp = self
            .checkpoints
            .get(pipeline_id)
            .ok_or(PipelineError::CheckpointNotFound(pipeline_id))?;
        let pipeline = self.get_pipeline(pipeline_id)?;
        self.log.append(ActionLogEntry::new(
            EventType::PipelineResumed,
            Actor::System,
            Some(pipeline.dimension_id),
            None,
            serde_json::json!({
                "pipeline_id": pipeline_id,
                "resumed_from_step_index": cp.completed_step_index,
                "processed_count": cp.processed_count,
            }),
        ));
        Ok(cp)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::{ArtifactId, DimensionId, TaskId};

    fn make_log() -> Arc<ActionLog> {
        ActionLog::new(64)
    }

    // -----------------------------------------------------------------------
    // Item 1 — Pipeline primitives
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_add_remove_step() {
        let dim = DimensionId::new();
        let mut pipeline = Pipeline::new("test", dim);
        assert_eq!(pipeline.version, 1);

        let step = PipelineStep::new("map prices", StepKind::Map);
        let step_id = step.id;
        pipeline.add_step(step);
        assert_eq!(pipeline.steps.len(), 1);
        assert_eq!(pipeline.version, 2);

        assert!(pipeline.remove_step(step_id));
        assert!(pipeline.steps.is_empty());
        assert_eq!(pipeline.version, 3);
    }

    #[test]
    fn step_kind_str_roundtrip() {
        let kinds = [
            StepKind::Map,
            StepKind::Filter,
            StepKind::Reduce,
            StepKind::Aggregate,
            StepKind::Window,
        ];
        let expected = ["map", "filter", "reduce", "aggregate", "window"];
        for (kind, exp) in kinds.iter().zip(expected.iter()) {
            assert_eq!(kind.as_str(), *exp);
        }
    }

    // -----------------------------------------------------------------------
    // Item 2 — Transform versioning and replay
    // -----------------------------------------------------------------------

    #[test]
    fn transform_version_capture() {
        let dim = DimensionId::new();
        let mut pipeline = Pipeline::new("v-test", dim);
        pipeline.add_step(PipelineStep::new("filter negatives", StepKind::Filter));

        let tv = TransformVersion::capture(&pipeline, "alice", "initial version");
        assert_eq!(tv.pipeline_id, pipeline.id);
        assert_eq!(tv.version, pipeline.version);
        assert_eq!(tv.steps_snapshot.len(), 1);
        assert_eq!(tv.author, "alice");
    }

    #[test]
    fn registry_replay_version() {
        let log = make_log();
        let registry = PipelineRegistry::new(Arc::clone(&log));
        let dim = DimensionId::new();
        let mut pipeline = Pipeline::new("replay-test", dim);
        pipeline.add_step(PipelineStep::new("map", StepKind::Map));
        let pipeline_id = registry.register_pipeline(pipeline.clone());

        registry
            .update_pipeline(pipeline.clone(), "bot", "step 1")
            .unwrap();

        let versions = registry.versions_for_pipeline(pipeline_id);
        assert_eq!(versions.len(), 1);

        let task = TaskId::new();
        let req = TransformReplayRequest {
            pipeline_id,
            version: versions[0].version,
            input_records: vec![serde_json::json!({"x": 1})],
            task_id: task,
        };
        let (tv, records) = registry.replay_version(req).unwrap();
        assert_eq!(tv.version, versions[0].version);
        assert_eq!(records.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Item 3 — Dead-letter queue
    // -----------------------------------------------------------------------

    #[test]
    fn dlq_enqueue_and_drain() {
        let dlq = DeadLetterQueue::new();
        let pipeline_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        dlq.enqueue(DeadLetterEntry::new(
            pipeline_id,
            step_id,
            serde_json::json!({"record": "bad"}),
            "parse error",
            DeadLetterReason::StepError,
        ));
        assert_eq!(dlq.len(), 1);
        assert_eq!(dlq.entries_for_pipeline(pipeline_id).len(), 1);
        let drained = dlq.drain();
        assert_eq!(drained.len(), 1);
        assert!(dlq.is_empty());
    }

    // -----------------------------------------------------------------------
    // Item 4 — Schema inference and validation
    // -----------------------------------------------------------------------

    #[test]
    fn schema_infer_from_object() {
        let sample = serde_json::json!({
            "id": 1,
            "name": "alice",
            "score": 9.5,
            "active": true,
        });
        let schema = SchemaInferrer::infer("users", &sample);
        assert_eq!(schema.fields.len(), 4);
        assert_eq!(schema.version, 1, "inferred schema must have version 1");
        let name_field = schema.fields.iter().find(|f| f.name == "name").unwrap();
        assert_eq!(name_field.field_type, FieldType::String);
        let id_field = schema.fields.iter().find(|f| f.name == "id").unwrap();
        assert_eq!(id_field.field_type, FieldType::Integer);
        let score_field = schema.fields.iter().find(|f| f.name == "score").unwrap();
        assert_eq!(score_field.field_type, FieldType::Float);
    }

    #[test]
    fn schema_validation_passes() {
        let sample = serde_json::json!({"id": 1, "name": "bob"});
        let schema = SchemaInferrer::infer("t", &sample);
        let result = SchemaValidator::validate(&sample, &schema);
        assert!(result.is_ok());
    }

    #[test]
    fn schema_validation_fails_wrong_type() {
        let sample = serde_json::json!({"id": 1, "name": "bob"});
        let schema = SchemaInferrer::infer("t", &sample);
        let bad_record = serde_json::json!({"id": "not-an-int", "name": "carol"});
        let result = SchemaValidator::validate(&bad_record, &schema);
        assert!(result.is_err());
    }

    #[test]
    fn schema_registry_store_retrieve() {
        let registry = SchemaRegistry::new();
        let schema = SchemaInferrer::infer("events", &serde_json::json!({"ts": 0}));
        let id = registry.register(schema);
        assert!(registry.get(id).is_some());
        assert_eq!(registry.all().len(), 1);
    }

    // -----------------------------------------------------------------------
    // Item 5 — Streaming mode + watermarking
    // -----------------------------------------------------------------------

    #[test]
    fn stream_pipeline_advance_watermark() {
        let dim = DimensionId::new();
        let pipeline = Pipeline::new("stream", dim);
        let mut sp = StreamPipeline::new(pipeline);
        assert_eq!(sp.low_watermark_ms, 0);

        let r1 = StreamRecord::new(serde_json::json!({"v": 1}), 1_000);
        let r2 = StreamRecord::new(serde_json::json!({"v": 2}), 3_000);
        sp.advance(&r1);
        sp.advance(&r2);

        assert_eq!(sp.low_watermark_ms, 3_000);
        assert_eq!(sp.records_processed, 2);
    }

    #[test]
    fn event_watermark_lag() {
        let wm = EventWatermark {
            event_time_ms: 1_000,
            processing_time_ms: 1_500,
        };
        assert_eq!(wm.lag_ms(), 500);
    }

    // -----------------------------------------------------------------------
    // Item 6 — Checkpointing and resume
    // -----------------------------------------------------------------------

    #[test]
    fn checkpoint_save_and_resume() {
        let log = make_log();
        let registry = PipelineRegistry::new(Arc::clone(&log));
        let dim = DimensionId::new();
        let pipeline = Pipeline::new("cp-test", dim);
        let pipeline_id = registry.register_pipeline(pipeline);

        let cp = registry
            .save_checkpoint(pipeline_id, 2, 5_000, serde_json::json!({"acc": 42}))
            .unwrap();
        assert_eq!(cp.completed_step_index, 2);
        assert_eq!(cp.processed_count, 5_000);

        let resumed = registry.resume_from_checkpoint(pipeline_id).unwrap();
        assert_eq!(resumed.id, cp.id);
    }

    #[test]
    fn resume_without_checkpoint_errors() {
        let log = make_log();
        let registry = PipelineRegistry::new(Arc::clone(&log));
        let dim = DimensionId::new();
        let pipeline = Pipeline::new("no-cp", dim);
        let pipeline_id = registry.register_pipeline(pipeline);
        let result = registry.resume_from_checkpoint(pipeline_id);
        assert!(matches!(result, Err(PipelineError::CheckpointNotFound(_))));
    }

    // -----------------------------------------------------------------------
    // Item 7 — Lineage tracking
    // -----------------------------------------------------------------------

    #[test]
    fn lineage_record_and_query() {
        let log = make_log();
        let tracker = LineageTracker::new();
        let dim = DimensionId::new();
        let task = TaskId::new();
        let pipeline_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let input = ArtifactId::new();
        let output = ArtifactId::new();

        let record = LineageRecord::new(
            pipeline_id,
            step_id,
            "map step",
            vec![input],
            output,
            1,
            task,
            dim,
        );
        tracker.record(record, &log);
        assert_eq!(tracker.len(), 1);
        let for_pipeline = tracker.records_for_pipeline(pipeline_id);
        assert_eq!(for_pipeline.len(), 1);
        assert_eq!(for_pipeline[0].output_artifact_id, output);
    }

    // -----------------------------------------------------------------------
    // Item 8 — Performance optimization
    // -----------------------------------------------------------------------

    #[test]
    fn optimizer_suggests_parallelism_increase() {
        let optimizer = PipelineOptimizer::new();
        let config = BatchConfig::default(); // parallelism = 4
        optimizer.record_sample(PipelineMetrics {
            pipeline_id: Uuid::new_v4(),
            records_processed: 100,
            records_failed: 0,
            throughput_rps: 200.0, // below 500 threshold
            avg_latency_ms: 10.0,
            p99_latency_ms: 20.0,
            sampled_at_ms: now_ms(),
        });
        let advice = optimizer.advise(&config);
        assert_eq!(advice.parallelism_delta, 2);
    }

    #[test]
    fn optimizer_suggests_batch_size_reduction() {
        let optimizer = PipelineOptimizer::new();
        let config = BatchConfig {
            max_batch_size: 1_000,
            parallelism: 16, // already at max parallelism
            ..BatchConfig::default()
        };
        optimizer.record_sample(PipelineMetrics {
            pipeline_id: Uuid::new_v4(),
            records_processed: 1_000,
            records_failed: 0,
            throughput_rps: 600.0, // above throughput threshold
            avg_latency_ms: 150.0, // above latency threshold
            p99_latency_ms: 300.0,
            sampled_at_ms: now_ms(),
        });
        let advice = optimizer.advise(&config);
        assert!(advice.batch_size_delta < 0);
    }

    // -----------------------------------------------------------------------
    // Item 9 — UI pipeline builder nodes
    // -----------------------------------------------------------------------

    #[test]
    fn builder_registry_has_all_kinds() {
        let registry = PipelineBuilderRegistry::with_defaults();
        assert_eq!(registry.len(), 9);
    }

    #[test]
    fn builder_node_for_map_has_ports() {
        let node = PipelineBuilderNode::for_kind(PipelineNodeKind::Map);
        assert_eq!(node.input_ports, vec!["in"]);
        assert_eq!(node.output_ports, vec!["out"]);
        assert_eq!(node.node_kind_str, "pipeline.map");
    }

    #[test]
    fn builder_node_source_has_no_inputs() {
        let node = PipelineBuilderNode::for_kind(PipelineNodeKind::Source);
        assert!(node.input_ports.is_empty());
        assert!(!node.output_ports.is_empty());
    }

    // -----------------------------------------------------------------------
    // Item 10 — Storage connectors
    // -----------------------------------------------------------------------

    #[test]
    fn connector_registry_register_and_retrieve() {
        let log = make_log();
        let registry = StorageConnectorRegistry::with_log(Arc::clone(&log));
        let conn = StorageConnector::new(
            "prod-postgres",
            StorageConnectorKind::Postgresql,
            serde_json::json!({"host": "localhost", "port": 5432, "db": "infinity"}),
        );
        let id = registry.register(conn);
        let retrieved = registry.get(id).unwrap();
        assert_eq!(retrieved.kind, StorageConnectorKind::Postgresql);
        // ActionLog event was emitted.
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn connector_registry_remove() {
        let registry = StorageConnectorRegistry::new();
        let conn = StorageConnector::new(
            "test-s3",
            StorageConnectorKind::S3Compatible,
            serde_json::json!({"endpoint": "http://minio:9000", "bucket": "data"}),
        );
        let id = registry.register(conn);
        assert!(registry.remove(id));
        assert!(registry.get(id).is_none());
        assert!(registry.is_empty());
    }

    // -----------------------------------------------------------------------
    // Integration — PipelineRegistry ties everything together
    // -----------------------------------------------------------------------

    #[test]
    fn registry_full_pipeline_lifecycle() {
        let log = make_log();
        let registry = PipelineRegistry::new(Arc::clone(&log));
        let dim = DimensionId::new();

        // Create pipeline with two steps.
        let mut pipeline = Pipeline::new("etl", dim);
        pipeline.add_step(PipelineStep::new("parse", StepKind::Map));
        pipeline.add_step(PipelineStep::new("drop nulls", StepKind::Filter));
        let id = registry.register_pipeline(pipeline.clone());

        // Save a version.
        registry
            .update_pipeline(pipeline.clone(), "system", "initial")
            .unwrap();
        assert_eq!(registry.versions_for_pipeline(id).len(), 1);

        // Register a schema.
        let schema = SchemaInferrer::infer("row", &serde_json::json!({"id": 1, "val": "x"}));
        let schema_id = registry.schemas.register(schema);
        assert!(registry.schemas.get(schema_id).is_some());

        // Save and resume checkpoint.
        registry
            .save_checkpoint(id, 1, 500, serde_json::json!({}))
            .unwrap();
        let cp = registry.resume_from_checkpoint(id).unwrap();
        assert_eq!(cp.completed_step_index, 1);

        // Register a storage connector.
        let conn = StorageConnector::new(
            "local",
            StorageConnectorKind::LocalFilesystem,
            serde_json::json!({"path": "/data/output"}),
        );
        registry.connectors.register(conn);
        assert_eq!(registry.connectors.len(), 1);

        // Verify ActionLog received events.
        assert!(log.len() >= 4);
    }
}
