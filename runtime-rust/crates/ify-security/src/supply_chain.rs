//! Supply chain protections — Epic O item 8.
//!
//! Provides [`Sbom`] (Software Bill of Materials) and
//! [`ComponentRecord`] for tracking all direct and transitive dependencies,
//! together with [`SupplyChainVerifier`] that checks component signatures
//! before installation.
//!
//! The SBOM is designed to be serialised as JSON and published alongside
//! release artifacts.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the supply chain subsystem.
#[derive(Debug, Error)]
pub enum SupplyChainError {
    /// A component's signature could not be verified.
    #[error("signature verification failed for component '{name}' v{version}")]
    SignatureInvalid {
        /// Component name.
        name: String,
        /// Component version.
        version: String,
    },
    /// A component with this name+version is already in the SBOM.
    #[error("component '{name}' v{version} already registered")]
    DuplicateComponent {
        /// Component name.
        name: String,
        /// Component version.
        version: String,
    },
    /// The required signature is missing from the component record.
    #[error("component '{name}' v{version} has no signature")]
    MissingSignature {
        /// Component name.
        name: String,
        /// Component version.
        version: String,
    },
}

// ---------------------------------------------------------------------------
// ComponentKind
// ---------------------------------------------------------------------------

/// Classifies the type of component in the SBOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// A library or crate dependency.
    Library,
    /// A container image.
    ContainerImage,
    /// A kernel module.
    KernelModule,
    /// An OS package.
    OsPackage,
    /// An ML model artifact.
    ModelArtifact,
    /// An agent template bundle.
    AgentTemplate,
}

// ---------------------------------------------------------------------------
// ComponentRecord
// ---------------------------------------------------------------------------

/// A single component entry in the SBOM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentRecord {
    /// Unique record ID.
    pub id: Uuid,
    /// Component name (e.g. `"tokio"`).
    pub name: String,
    /// Version string (SemVer).
    pub version: String,
    /// Ecosystem / package manager.
    pub ecosystem: String,
    /// Kind of component.
    pub kind: ComponentKind,
    /// Source URL (registry, git repo, etc.).
    pub source_url: Option<String>,
    /// Hex-encoded SHA-256 content hash of the component archive.
    pub content_hash: Option<String>,
    /// Hex-encoded digital signature of the content hash.
    pub signature: Option<String>,
    /// Identifier of the key used to produce the signature.
    pub signature_key_id: Option<String>,
    /// Known CVE identifiers for this version, if any.
    pub known_vulnerabilities: Vec<String>,
}

impl ComponentRecord {
    /// Create a minimal component record.
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        ecosystem: impl Into<String>,
        kind: ComponentKind,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            version: version.into(),
            ecosystem: ecosystem.into(),
            kind,
            source_url: None,
            content_hash: None,
            signature: None,
            signature_key_id: None,
            known_vulnerabilities: Vec::new(),
        }
    }

    /// Attach a source URL.
    pub fn with_source(mut self, url: impl Into<String>) -> Self {
        self.source_url = Some(url.into());
        self
    }

    /// Attach a content hash and signature.
    pub fn with_signature(
        mut self,
        content_hash: impl Into<String>,
        signature: impl Into<String>,
        key_id: impl Into<String>,
    ) -> Self {
        self.content_hash = Some(content_hash.into());
        self.signature = Some(signature.into());
        self.signature_key_id = Some(key_id.into());
        self
    }

    /// Mark known CVEs.
    pub fn with_vulnerabilities(mut self, cves: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.known_vulnerabilities = cves.into_iter().map(Into::into).collect();
        self
    }
}

// ---------------------------------------------------------------------------
// Sbom
// ---------------------------------------------------------------------------

/// Software Bill of Materials for an infinityOS release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sbom {
    /// SBOM identifier.
    pub sbom_id: Uuid,
    /// Release or build label (e.g. `"v0.1.0-alpha1"`).
    pub release_label: String,
    /// ISO-8601 generation timestamp.
    pub generated_at: String,
    /// Map of component ID → record.
    components: HashMap<Uuid, ComponentRecord>,
}

impl Sbom {
    /// Create an empty SBOM for `release_label`.
    pub fn new(release_label: impl Into<String>, generated_at: impl Into<String>) -> Self {
        Self {
            sbom_id: Uuid::new_v4(),
            release_label: release_label.into(),
            generated_at: generated_at.into(),
            components: HashMap::new(),
        }
    }

    /// Add a component to the SBOM.
    ///
    /// # Errors
    ///
    /// Returns [`SupplyChainError::DuplicateComponent`] if a component with
    /// the same `name` + `version` is already registered.
    pub fn add_component(&mut self, record: ComponentRecord) -> Result<Uuid, SupplyChainError> {
        let duplicate = self
            .components
            .values()
            .any(|c| c.name == record.name && c.version == record.version);
        if duplicate {
            return Err(SupplyChainError::DuplicateComponent {
                name: record.name,
                version: record.version,
            });
        }
        let id = record.id;
        self.components.insert(id, record);
        Ok(id)
    }

    /// Look up a component by ID.
    pub fn get(&self, id: Uuid) -> Option<&ComponentRecord> {
        self.components.get(&id)
    }

    /// Iterate over all component records.
    pub fn components(&self) -> impl Iterator<Item = &ComponentRecord> {
        self.components.values()
    }

    /// Total number of components.
    pub fn component_count(&self) -> usize {
        self.components.len()
    }

    /// Return all components with known vulnerabilities.
    pub fn vulnerable_components(&self) -> Vec<&ComponentRecord> {
        self.components
            .values()
            .filter(|c| !c.known_vulnerabilities.is_empty())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// SupplyChainVerifier
// ---------------------------------------------------------------------------

/// Verifies component signatures before installation.
///
/// The verifier uses the same FNV-mixing scheme as [`ArtifactSigner`].
/// Replace with Ed25519 verification for production deployments.
pub struct SupplyChainVerifier {
    /// Map of key_id → key bytes.
    trusted_keys: HashMap<String, Vec<u8>>,
}

impl SupplyChainVerifier {
    /// Create a verifier with no trusted keys.
    pub fn new() -> Self {
        Self { trusted_keys: HashMap::new() }
    }

    /// Register a trusted public key.
    pub fn add_trusted_key(&mut self, key_id: impl Into<String>, key_bytes: impl Into<Vec<u8>>) {
        self.trusted_keys.insert(key_id.into(), key_bytes.into());
    }

    /// Verify the signature on `record`.
    ///
    /// # Errors
    ///
    /// - [`SupplyChainError::MissingSignature`] — record has no signature.
    /// - [`SupplyChainError::SignatureInvalid`] — signature does not match.
    pub fn verify(&self, record: &ComponentRecord) -> Result<(), SupplyChainError> {
        let content_hash = record.content_hash.as_deref().ok_or_else(|| {
            SupplyChainError::MissingSignature {
                name: record.name.clone(),
                version: record.version.clone(),
            }
        })?;
        let signature = record.signature.as_deref().ok_or_else(|| {
            SupplyChainError::MissingSignature {
                name: record.name.clone(),
                version: record.version.clone(),
            }
        })?;
        let key_id = record.signature_key_id.as_deref().unwrap_or("");
        let key_bytes = self
            .trusted_keys
            .get(key_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let expected = Self::sign_hash(key_bytes, content_hash.as_bytes());
        if hex_encode(&expected) != signature {
            return Err(SupplyChainError::SignatureInvalid {
                name: record.name.clone(),
                version: record.version.clone(),
            });
        }
        Ok(())
    }

    fn sign_hash(key: &[u8], hash: &[u8]) -> [u8; 8] {
        let mut state: u64 = 0xcbf29ce484222325;
        for &b in key.iter().chain(hash.iter()) {
            state ^= u64::from(b);
            state = state.wrapping_mul(0x00000100000001b3);
        }
        state.to_le_bytes()
    }

    /// Verify all signed components in an SBOM.
    ///
    /// Returns a list of errors for any failing components.
    pub fn verify_sbom(&self, sbom: &Sbom) -> Vec<SupplyChainError> {
        sbom.components()
            .filter(|c| c.signature.is_some())
            .filter_map(|c| self.verify(c).err())
            .collect()
    }
}

impl Default for SupplyChainVerifier {
    fn default() -> Self {
        Self::new()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_record(key: &[u8], name: &str, version: &str) -> ComponentRecord {
        let content_hash = "abc123def456";
        // Compute expected signature.
        let mut state: u64 = 0xcbf29ce484222325;
        for &b in key.iter().chain(content_hash.as_bytes().iter()) {
            state ^= u64::from(b);
            state = state.wrapping_mul(0x00000100000001b3);
        }
        let sig = state
            .to_le_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();

        ComponentRecord::new(name, version, "crates.io", ComponentKind::Library)
            .with_signature(content_hash, sig, "key-v1")
    }

    #[test]
    fn valid_signature_passes() {
        let key = b"my-signing-key";
        let record = signed_record(key, "tokio", "1.0.0");
        let mut verifier = SupplyChainVerifier::new();
        verifier.add_trusted_key("key-v1", key.to_vec());
        assert!(verifier.verify(&record).is_ok());
    }

    #[test]
    fn wrong_key_fails() {
        let key = b"correct-key";
        let record = signed_record(key, "serde", "1.0.0");
        let mut verifier = SupplyChainVerifier::new();
        verifier.add_trusted_key("key-v1", b"wrong-key".to_vec());
        assert!(verifier.verify(&record).is_err());
    }

    #[test]
    fn missing_signature_fails() {
        let record = ComponentRecord::new("bare", "0.1.0", "crates.io", ComponentKind::Library);
        let verifier = SupplyChainVerifier::new();
        assert!(matches!(
            verifier.verify(&record),
            Err(SupplyChainError::MissingSignature { .. })
        ));
    }

    #[test]
    fn sbom_duplicate_component_rejected() {
        let mut sbom = Sbom::new("v0.1.0", "2026-03-11T00:00:00Z");
        let r1 = ComponentRecord::new("lib", "1.0.0", "crates.io", ComponentKind::Library);
        let r2 = ComponentRecord::new("lib", "1.0.0", "crates.io", ComponentKind::Library);
        sbom.add_component(r1).unwrap();
        assert!(matches!(
            sbom.add_component(r2),
            Err(SupplyChainError::DuplicateComponent { .. })
        ));
    }

    #[test]
    fn sbom_vulnerable_components_filter() {
        let mut sbom = Sbom::new("v0.1.0", "2026-03-11T00:00:00Z");
        let safe = ComponentRecord::new("safe-lib", "2.0.0", "crates.io", ComponentKind::Library);
        let vuln = ComponentRecord::new("vuln-lib", "0.1.0", "crates.io", ComponentKind::Library)
            .with_vulnerabilities(["CVE-2024-0001"]);
        sbom.add_component(safe).unwrap();
        sbom.add_component(vuln).unwrap();
        assert_eq!(sbom.vulnerable_components().len(), 1);
        assert_eq!(sbom.vulnerable_components()[0].name, "vuln-lib");
    }

    #[test]
    fn verify_sbom_all_pass() {
        let key = b"build-key";
        let mut sbom = Sbom::new("v0.1.0", "2026-03-11T00:00:00Z");
        sbom.add_component(signed_record(key, "crate-a", "1.0.0")).unwrap();
        sbom.add_component(signed_record(key, "crate-b", "2.0.0")).unwrap();
        let mut verifier = SupplyChainVerifier::new();
        verifier.add_trusted_key("key-v1", key.to_vec());
        assert!(verifier.verify_sbom(&sbom).is_empty());
    }
}
