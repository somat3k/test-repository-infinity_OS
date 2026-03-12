//! Memory subsystem — short-term, long-term, and vector-store hook.
//!
//! ## Overview
//!
//! The memory subsystem provides three layers of memory for agentic tasks:
//!
//! | Layer              | Scope            | Persistence | Description                        |
//! |--------------------|------------------|-------------|------------------------------------|
//! [`ShortTermMemory`]  | In-process       | Ephemeral   | Fast key/value store per task      |
//! [`LongTermMemory`]   | Cross-task       | Persistent  | Pluggable durable store (trait)    |
//! [`VectorStoreHook`]  | Semantic search  | Persistent  | Pluggable vector DB hook (trait)   |
//!
//! [`MemorySubsystem`] composes all three and exposes a unified API used by
//! the agent executor.
//!
//! ## Design
//!
//! - All operations are `async` to allow non-blocking I/O backends.
//! - Errors are returned as `Result` — no panics.
//! - `ShortTermMemory` is scoped to a `(DimensionId, TaskId)` pair so
//!   that memory isolation across tasks is trivially enforced.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ify_core::{DimensionId, TaskId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by memory subsystem operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MemoryError {
    /// A key was not found in the requested store.
    #[error("memory key not found: {0}")]
    KeyNotFound(String),

    /// Serialisation or deserialisation of a stored value failed.
    #[error("serialisation error: {0}")]
    Serialisation(String),

    /// A backend operation failed.
    #[error("backend error: {0}")]
    Backend(String),
}

// ---------------------------------------------------------------------------
// MemoryKey
// ---------------------------------------------------------------------------

/// A typed key for memory stores.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryKey(pub String);

impl MemoryKey {
    /// Create a new key.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Display for MemoryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for MemoryKey {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

// ---------------------------------------------------------------------------
// ShortTermMemory
// ---------------------------------------------------------------------------

/// In-process ephemeral key/value store scoped to a `(DimensionId, TaskId)`.
///
/// Values are stored as [`serde_json::Value`] so that heterogeneous payloads
/// can coexist in the same store without a common base type.
pub struct ShortTermMemory {
    dimension_id: DimensionId,
    task_id: TaskId,
    store: RwLock<HashMap<MemoryKey, serde_json::Value>>,
}

impl ShortTermMemory {
    /// Create an empty short-term memory scoped to the given task.
    pub fn new(dimension_id: DimensionId, task_id: TaskId) -> Self {
        Self {
            dimension_id,
            task_id,
            store: RwLock::new(HashMap::new()),
        }
    }

    /// Return the owning dimension.
    pub fn dimension_id(&self) -> DimensionId {
        self.dimension_id
    }

    /// Return the owning task.
    pub fn task_id(&self) -> TaskId {
        self.task_id
    }

    /// Store a value.
    #[instrument(skip(self, value, key))]
    pub async fn set(&self, key: impl Into<MemoryKey>, value: serde_json::Value) {
        let key = key.into();
        debug!(key = %key, "short-term memory set");
        self.store.write().await.insert(key, value);
    }

    /// Retrieve a value.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::KeyNotFound`] if the key does not exist.
    #[instrument(skip(self, key))]
    pub async fn get(&self, key: impl Into<MemoryKey>) -> Result<serde_json::Value, MemoryError> {
        let key = key.into();
        self.store
            .read()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| MemoryError::KeyNotFound(key.0.clone()))
    }

    /// Remove a key, returning the previous value if present.
    pub async fn remove(&self, key: impl Into<MemoryKey>) -> Option<serde_json::Value> {
        self.store.write().await.remove(&key.into())
    }

    /// Return all keys currently in the store.
    pub async fn keys(&self) -> Vec<MemoryKey> {
        self.store.read().await.keys().cloned().collect()
    }

    /// Clear all entries.
    pub async fn clear(&self) {
        self.store.write().await.clear();
    }

    /// Return the number of stored entries.
    pub async fn len(&self) -> usize {
        self.store.read().await.len()
    }

    /// Return `true` if no entries are stored.
    pub async fn is_empty(&self) -> bool {
        self.store.read().await.is_empty()
    }
}

// ---------------------------------------------------------------------------
// LongTermMemory trait
// ---------------------------------------------------------------------------

/// Trait for persistent (cross-task) memory backends.
///
/// Implementations may wrap a database, a remote key/value store, or a
/// filesystem-backed store.  The default [`NoopLongTermMemory`] is a no-op
/// used when no persistent backend is configured.
pub trait LongTermMemory: Send + Sync {
    /// Persist a value.
    fn put(
        &self,
        dimension_id: DimensionId,
        key: MemoryKey,
        value: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>>;

    /// Retrieve a persisted value.
    fn get(
        &self,
        dimension_id: DimensionId,
        key: &MemoryKey,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, MemoryError>> + Send + '_>>;

    /// Delete a key.
    fn delete(
        &self,
        dimension_id: DimensionId,
        key: &MemoryKey,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>>;
}

/// No-op long-term memory backend (always returns [`MemoryError::Backend`]).
///
/// Used as a default when no persistent backend is configured.
pub struct NoopLongTermMemory;

impl LongTermMemory for NoopLongTermMemory {
    fn put(
        &self,
        _dimension_id: DimensionId,
        _key: MemoryKey,
        _value: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>> {
        Box::pin(async { Err(MemoryError::Backend("no long-term memory backend configured".to_owned())) })
    }

    fn get(
        &self,
        _dimension_id: DimensionId,
        _key: &MemoryKey,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, MemoryError>> + Send + '_>> {
        Box::pin(async { Err(MemoryError::Backend("no long-term memory backend configured".to_owned())) })
    }

    fn delete(
        &self,
        _dimension_id: DimensionId,
        _key: &MemoryKey,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>> {
        Box::pin(async { Err(MemoryError::Backend("no long-term memory backend configured".to_owned())) })
    }
}

// ---------------------------------------------------------------------------
// VectorStoreHook trait
// ---------------------------------------------------------------------------

/// A single document stored in the vector store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorDocument {
    /// Opaque document identifier.
    pub id: String,
    /// Embedding vector.
    pub embedding: Vec<f32>,
    /// Arbitrary metadata.
    pub metadata: serde_json::Value,
}

/// A query result from a vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorMatch {
    /// The matched document.
    pub document: VectorDocument,
    /// Cosine similarity score in [0.0, 1.0].
    pub score: f32,
}

/// Trait for plugging in a vector store backend.
///
/// Implement this to connect an external embedding database (e.g., Qdrant,
/// Weaviate, pgvector) to the agent memory subsystem.
pub trait VectorStoreHook: Send + Sync {
    /// Upsert a document into the store.
    fn upsert(
        &self,
        dimension_id: DimensionId,
        doc: VectorDocument,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>>;

    /// Perform approximate-nearest-neighbour search.
    fn search(
        &self,
        dimension_id: DimensionId,
        query: Vec<f32>,
        top_k: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<VectorMatch>, MemoryError>> + Send + '_>>;

    /// Delete a document by ID.
    fn delete(
        &self,
        dimension_id: DimensionId,
        doc_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>>;
}

/// No-op vector store hook used when no backend is configured.
pub struct NoopVectorStore;

impl VectorStoreHook for NoopVectorStore {
    fn upsert(
        &self,
        _dimension_id: DimensionId,
        _doc: VectorDocument,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>> {
        Box::pin(async { Err(MemoryError::Backend("no vector store backend configured".to_owned())) })
    }

    fn search(
        &self,
        _dimension_id: DimensionId,
        _query: Vec<f32>,
        _top_k: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<VectorMatch>, MemoryError>> + Send + '_>> {
        Box::pin(async { Err(MemoryError::Backend("no vector store backend configured".to_owned())) })
    }

    fn delete(
        &self,
        _dimension_id: DimensionId,
        _doc_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MemoryError>> + Send + '_>> {
        Box::pin(async { Err(MemoryError::Backend("no vector store backend configured".to_owned())) })
    }
}

// ---------------------------------------------------------------------------
// MemorySubsystem
// ---------------------------------------------------------------------------

/// Configuration for the [`MemorySubsystem`].
#[derive(Debug, Default)]
pub struct MemorySubsystemConfig {
    // Future: add max short-term entries, eviction policy, etc.
}

/// Composed memory subsystem integrating short-term, long-term, and vector memory.
pub struct MemorySubsystem {
    _config: MemorySubsystemConfig,
    long_term: Arc<dyn LongTermMemory>,
    vector_store: Arc<dyn VectorStoreHook>,
}

impl MemorySubsystem {
    /// Create a new subsystem with the default (no-op) backends.
    pub fn new(config: MemorySubsystemConfig) -> Self {
        Self {
            _config: config,
            long_term: Arc::new(NoopLongTermMemory),
            vector_store: Arc::new(NoopVectorStore),
        }
    }

    /// Create a new subsystem with custom backends.
    pub fn with_backends(
        config: MemorySubsystemConfig,
        long_term: Arc<dyn LongTermMemory>,
        vector_store: Arc<dyn VectorStoreHook>,
    ) -> Self {
        Self {
            _config: config,
            long_term,
            vector_store,
        }
    }

    /// Allocate a new short-term memory store scoped to the given task.
    pub fn short_term(&self, dimension_id: DimensionId, task_id: TaskId) -> ShortTermMemory {
        ShortTermMemory::new(dimension_id, task_id)
    }

    /// Access the long-term memory backend.
    pub fn long_term(&self) -> &dyn LongTermMemory {
        self.long_term.as_ref()
    }

    /// Access the vector store hook.
    pub fn vector_store(&self) -> &dyn VectorStoreHook {
        self.vector_store.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ify_core::{DimensionId, TaskId};

    #[tokio::test]
    async fn short_term_set_get() {
        let dim = DimensionId::new();
        let task = TaskId::new();
        let mem = ShortTermMemory::new(dim, task);

        mem.set("answer", serde_json::json!(42)).await;
        let v = mem.get("answer").await.unwrap();
        assert_eq!(v, serde_json::json!(42));
    }

    #[tokio::test]
    async fn short_term_get_missing_key() {
        let mem = ShortTermMemory::new(DimensionId::new(), TaskId::new());
        let err = mem.get("ghost").await.unwrap_err();
        assert!(matches!(err, MemoryError::KeyNotFound(_)));
    }

    #[tokio::test]
    async fn short_term_remove() {
        let mem = ShortTermMemory::new(DimensionId::new(), TaskId::new());
        mem.set("x", serde_json::json!(1)).await;
        let removed = mem.remove("x").await;
        assert_eq!(removed, Some(serde_json::json!(1)));
        assert!(mem.is_empty().await);
    }

    #[tokio::test]
    async fn short_term_keys_and_len() {
        let mem = ShortTermMemory::new(DimensionId::new(), TaskId::new());
        mem.set("a", serde_json::json!(1)).await;
        mem.set("b", serde_json::json!(2)).await;
        assert_eq!(mem.len().await, 2);
        let mut keys: Vec<String> = mem.keys().await.into_iter().map(|k| k.0).collect();
        keys.sort();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn short_term_clear() {
        let mem = ShortTermMemory::new(DimensionId::new(), TaskId::new());
        mem.set("a", serde_json::json!(1)).await;
        mem.clear().await;
        assert!(mem.is_empty().await);
    }

    #[tokio::test]
    async fn noop_long_term_returns_error() {
        let subsys = MemorySubsystem::new(MemorySubsystemConfig::default());
        let dim = DimensionId::new();
        let err = subsys
            .long_term()
            .get(dim, &MemoryKey::new("k"))
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::Backend(_)));
    }

    #[tokio::test]
    async fn noop_vector_store_returns_error() {
        let subsys = MemorySubsystem::new(MemorySubsystemConfig::default());
        let dim = DimensionId::new();
        let err = subsys
            .vector_store()
            .search(dim, vec![0.0; 4], 3)
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::Backend(_)));
    }

    #[test]
    fn memory_key_display() {
        let k = MemoryKey::new("hello");
        assert_eq!(k.to_string(), "hello");
    }

    #[tokio::test]
    async fn subsystem_short_term_is_fresh_per_call() {
        let subsys = MemorySubsystem::new(MemorySubsystemConfig::default());
        let dim = DimensionId::new();
        let task = TaskId::new();
        let st1 = subsys.short_term(dim, task);
        let st2 = subsys.short_term(dim, task);
        st1.set("x", serde_json::json!(99)).await;
        // st2 is a separate allocation — should not see st1's entries
        assert!(st2.get("x").await.is_err());
    }
}
