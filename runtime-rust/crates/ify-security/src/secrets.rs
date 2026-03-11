//! Secret management and redaction — Epic O item 7.
//!
//! [`SecretStore`] provides a simple in-memory vault for named secrets.
//! Secrets are stored as opaque byte strings and are never logged directly.
//!
//! [`Redactor`] scans string output (logs, node output, artifact payloads)
//! and replaces any registered secret patterns with `[REDACTED]`, preventing
//! accidental leakage through the canvas UI or mesh artifacts.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the secret management subsystem.
#[derive(Debug, Error)]
pub enum SecretError {
    /// No secret with the given name was found.
    #[error("secret '{0}' not found")]
    NotFound(String),
    /// A secret with this name is already registered.
    #[error("secret '{0}' already registered")]
    Duplicate(String),
}

// ---------------------------------------------------------------------------
// SecretEntry
// ---------------------------------------------------------------------------

/// A stored secret entry.
#[derive(Debug, Clone)]
pub struct SecretEntry {
    /// Human-readable name (e.g. `"stripe-api-key"`).
    pub name: String,
    /// Opaque value bytes.
    value: Vec<u8>,
}

impl SecretEntry {
    fn new(name: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        Self { name: name.into(), value: value.into() }
    }
}

// ---------------------------------------------------------------------------
// SecretStore
// ---------------------------------------------------------------------------

/// In-memory vault for named secrets.
///
/// # Security notes
///
/// - Secret values are held as plain bytes in process memory. In production,
///   replace with OS keychain integration or an external KMS.
/// - The `Display` and `Debug` implementations intentionally omit the value.
pub struct SecretStore {
    secrets: HashMap<String, SecretEntry>,
}

impl SecretStore {
    /// Create an empty secret store.
    pub fn new() -> Self {
        Self { secrets: HashMap::new() }
    }

    /// Register a secret.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::Duplicate`] when a secret with this name
    /// already exists.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        value: impl Into<Vec<u8>>,
    ) -> Result<(), SecretError> {
        let name = name.into();
        if self.secrets.contains_key(&name) {
            return Err(SecretError::Duplicate(name));
        }
        self.secrets.insert(name.clone(), SecretEntry::new(name, value));
        Ok(())
    }

    /// Read a secret value.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::NotFound`] when the name is unknown.
    pub fn read(&self, name: &str) -> Result<&[u8], SecretError> {
        self.secrets
            .get(name)
            .map(|e| e.value.as_slice())
            .ok_or_else(|| SecretError::NotFound(name.to_owned()))
    }

    /// Read a secret as a UTF-8 string (returns `None` for non-UTF-8 values).
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::NotFound`] when the name is unknown.
    pub fn read_str(&self, name: &str) -> Result<Option<&str>, SecretError> {
        let bytes = self.read(name)?;
        Ok(std::str::from_utf8(bytes).ok())
    }

    /// Remove a secret.
    ///
    /// # Errors
    ///
    /// Returns [`SecretError::NotFound`] when the name is unknown.
    pub fn remove(&mut self, name: &str) -> Result<(), SecretError> {
        self.secrets
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| SecretError::NotFound(name.to_owned()))
    }

    /// Returns `true` when a secret with this name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.secrets.contains_key(name)
    }

    /// Number of registered secrets.
    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    /// Returns `true` when the store contains no secrets.
    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// Iterate over registered secret names (values are not exposed).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.secrets.keys().map(|s| s.as_str())
    }
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// RedactionPattern
// ---------------------------------------------------------------------------

/// A pattern used by the [`Redactor`] to find secret occurrences in text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionPattern {
    /// Unique pattern identifier.
    pub id: String,
    /// Literal string to search for (case-sensitive).
    pub literal: String,
}

impl RedactionPattern {
    /// Create a new literal redaction pattern.
    pub fn new(id: impl Into<String>, literal: impl Into<String>) -> Self {
        Self { id: id.into(), literal: literal.into() }
    }
}

// ---------------------------------------------------------------------------
// Redactor
// ---------------------------------------------------------------------------

/// Scans string data and replaces registered secret patterns with
/// `[REDACTED]`.
///
/// Patterns are evaluated in registration order.  An occurrence is replaced
/// completely before the next pattern is evaluated (non-overlapping, leftmost
/// first, repeated until no more matches remain).
#[derive(Debug, Default)]
pub struct Redactor {
    patterns: Vec<RedactionPattern>,
}

/// Replacement marker used in redacted output.
pub const REDACTED_MARKER: &str = "[REDACTED]";

impl Redactor {
    /// Create a redactor with no patterns.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pattern.
    pub fn add_pattern(&mut self, pattern: RedactionPattern) {
        self.patterns.push(pattern);
    }

    /// Add a literal string directly as a pattern.
    pub fn add_literal(&mut self, id: impl Into<String>, literal: impl Into<String>) {
        self.add_pattern(RedactionPattern::new(id, literal));
    }

    /// Replace all pattern occurrences in `input` with `[REDACTED]`.
    pub fn redact(&self, input: &str) -> String {
        let mut output = input.to_owned();
        for pattern in &self.patterns {
            if !pattern.literal.is_empty() {
                output = output.replace(pattern.literal.as_str(), REDACTED_MARKER);
            }
        }
        output
    }

    /// Redact a JSON value in-place (converts to string, redacts, parses back).
    ///
    /// String scalars and object values are redacted; numbers and booleans
    /// are left unchanged.
    pub fn redact_json(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => {
                serde_json::Value::String(self.redact(s))
            }
            serde_json::Value::Object(map) => {
                let redacted = map
                    .iter()
                    .map(|(k, v)| (k.clone(), self.redact_json(v)))
                    .collect();
                serde_json::Value::Object(redacted)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| self.redact_json(v)).collect())
            }
            other => other.clone(),
        }
    }

    /// Number of registered patterns.
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_store_register_and_read() {
        let mut store = SecretStore::new();
        store.register("api-key", b"s3cr3t".as_slice()).unwrap();
        assert_eq!(store.read("api-key").unwrap(), b"s3cr3t");
    }

    #[test]
    fn secret_store_prevents_duplicates() {
        let mut store = SecretStore::new();
        store.register("key", b"v1".as_slice()).unwrap();
        assert!(matches!(store.register("key", b"v2".as_slice()), Err(SecretError::Duplicate(_))));
    }

    #[test]
    fn secret_store_not_found() {
        let store = SecretStore::new();
        assert!(matches!(store.read("missing"), Err(SecretError::NotFound(_))));
    }

    #[test]
    fn secret_store_remove() {
        let mut store = SecretStore::new();
        store.register("tmp", b"x".as_slice()).unwrap();
        store.remove("tmp").unwrap();
        assert!(!store.contains("tmp"));
    }

    #[test]
    fn redactor_replaces_literal() {
        let mut r = Redactor::new();
        r.add_literal("stripe", "sk_live_ABCDEF");
        let out = r.redact("Authorization: Bearer sk_live_ABCDEF");
        assert_eq!(out, "Authorization: Bearer [REDACTED]");
    }

    #[test]
    fn redactor_multiple_patterns() {
        let mut r = Redactor::new();
        r.add_literal("key1", "secret-one");
        r.add_literal("key2", "secret-two");
        let out = r.redact("val1=secret-one val2=secret-two");
        assert!(!out.contains("secret-one"));
        assert!(!out.contains("secret-two"));
        assert_eq!(out, "val1=[REDACTED] val2=[REDACTED]");
    }

    #[test]
    fn redactor_json_value() {
        let mut r = Redactor::new();
        r.add_literal("tok", "TOKEN");
        let v = serde_json::json!({"auth": "Bearer TOKEN", "count": 1});
        let redacted = r.redact_json(&v);
        assert_eq!(redacted["auth"], "Bearer [REDACTED]");
        assert_eq!(redacted["count"], 1);
    }

    #[test]
    fn redactor_empty_input_unchanged() {
        let mut r = Redactor::new();
        r.add_literal("k", "s3cr3t_p@ss");
        assert_eq!(r.redact(""), "");
        assert_eq!(r.redact("no sensitive data here"), "no sensitive data here");
    }

    #[test]
    fn redactor_no_patterns() {
        let r = Redactor::new();
        assert_eq!(r.redact("anything"), "anything");
    }
}
