//! Signed artifacts for runtime and deploy paths — Epic O item 5.
//!
//! The [`ArtifactSigner`] signs artifact payloads before they are published
//! to the mesh or a deployment target.  [`SignedArtifact`] carries the
//! payload together with its [`ArtifactSignature`], and
//! [`ArtifactVerifier`] can re-derive the signature to confirm integrity.
//!
//! This implementation uses a keyed HMAC-like approach (FNV mixing of the
//! signing key and payload bytes) suitable for in-process use.  Production
//! deployments should replace the `sign_bytes` / `verify_bytes` internals
//! with a proper asymmetric scheme (e.g. Ed25519 via the `ed25519-dalek`
//! crate) without changing the public API.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by the artifact signing subsystem.
#[derive(Debug, Error)]
pub enum SigningError {
    /// Signature verification failed.
    #[error("signature verification failed for artifact '{artifact_id}'")]
    VerificationFailed {
        /// ID of the artifact that failed verification.
        artifact_id: Uuid,
    },
    /// The payload could not be serialized.
    #[error("serialization error: {0}")]
    Serialization(String),
}

// ---------------------------------------------------------------------------
// ArtifactSignature
// ---------------------------------------------------------------------------

/// A signature over an artifact payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactSignature {
    /// Hex-encoded signature bytes.
    pub value: String,
    /// Identifier of the key used to produce the signature.
    pub key_id: String,
    /// Algorithm identifier.
    pub algorithm: String,
}

// ---------------------------------------------------------------------------
// SignedArtifact
// ---------------------------------------------------------------------------

/// An artifact payload bundled with its signature metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedArtifact {
    /// Unique artifact identifier.
    pub artifact_id: Uuid,
    /// Raw artifact payload (JSON).
    pub payload: serde_json::Value,
    /// Signature produced by [`ArtifactSigner`].
    pub signature: ArtifactSignature,
}

// ---------------------------------------------------------------------------
// ArtifactSigner
// ---------------------------------------------------------------------------

/// Signs artifact payloads before mesh publication or deployment.
///
/// Each signer holds a logical key identifier and a secret signing key.
/// The key material is never serialized and never leaves the signer.
pub struct ArtifactSigner {
    key_id: String,
    /// Secret key bytes (held in-memory only).
    key_bytes: Vec<u8>,
}

impl ArtifactSigner {
    /// Create a new signer.
    ///
    /// `key_id` is a human-readable label (e.g. `"runtime-sign-v1"`).
    /// `key_bytes` is the raw key material.
    pub fn new(key_id: impl Into<String>, key_bytes: impl Into<Vec<u8>>) -> Self {
        Self { key_id: key_id.into(), key_bytes: key_bytes.into() }
    }

    /// Sign `payload` and return a [`SignedArtifact`].
    ///
    /// The payload is serialized using a canonical JSON representation
    /// (object keys sorted lexicographically at every nesting level) so
    /// that two semantically identical `Value`s always produce the same
    /// byte string, regardless of the insertion order used by the caller.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::Serialization`] if `payload` cannot be
    /// serialized.
    pub fn sign(
        &self,
        artifact_id: Uuid,
        payload: serde_json::Value,
    ) -> Result<SignedArtifact, SigningError> {
        let canonical = canonical_json_bytes(&payload)
            .map_err(|e| SigningError::Serialization(e))?;
        let sig_bytes = self.sign_bytes(&canonical);
        Ok(SignedArtifact {
            artifact_id,
            payload,
            signature: ArtifactSignature {
                value: hex_encode(&sig_bytes),
                key_id: self.key_id.clone(),
                algorithm: "hmac-fnv64".into(),
            },
        })
    }

    fn sign_bytes(&self, payload: &[u8]) -> [u8; 8] {
        // FNV-1a 64-bit mixing of key ++ payload — replace with HMAC-SHA256
        // or Ed25519 for production use.
        let mut state: u64 = 0xcbf29ce484222325;
        for &b in self.key_bytes.iter().chain(payload.iter()) {
            state ^= u64::from(b);
            state = state.wrapping_mul(0x00000100000001b3);
        }
        state.to_le_bytes()
    }
}

// ---------------------------------------------------------------------------
// ArtifactVerifier
// ---------------------------------------------------------------------------

/// Verifies the signature on a [`SignedArtifact`].
pub struct ArtifactVerifier {
    /// Must match the signer's key bytes.
    key_bytes: Vec<u8>,
}

impl ArtifactVerifier {
    /// Create a verifier with the matching key material.
    pub fn new(key_bytes: impl Into<Vec<u8>>) -> Self {
        Self { key_bytes: key_bytes.into() }
    }

    /// Verify the signature on `artifact`.
    ///
    /// The same canonical JSON serialisation (sorted keys) is used as during
    /// signing, so the check is stable regardless of how the `Value` was
    /// constructed.
    ///
    /// # Errors
    ///
    /// Returns [`SigningError::VerificationFailed`] when the recomputed
    /// signature does not match the stored one.
    pub fn verify(&self, artifact: &SignedArtifact) -> Result<(), SigningError> {
        let canonical = canonical_json_bytes(&artifact.payload)
            .map_err(|e| SigningError::Serialization(e))?;
        let expected = self.sign_bytes(&canonical);
        if hex_encode(&expected) != artifact.signature.value {
            return Err(SigningError::VerificationFailed {
                artifact_id: artifact.artifact_id,
            });
        }
        Ok(())
    }

    fn sign_bytes(&self, payload: &[u8]) -> [u8; 8] {
        let mut state: u64 = 0xcbf29ce484222325;
        for &b in self.key_bytes.iter().chain(payload.iter()) {
            state ^= u64::from(b);
            state = state.wrapping_mul(0x00000100000001b3);
        }
        state.to_le_bytes()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Produce a deterministic canonical JSON byte string by sorting object keys
/// lexicographically at every nesting level.
///
/// This ensures that two semantically identical `serde_json::Value` objects
/// that differ only in map-key insertion order produce identical byte strings,
/// which is required for stable signature verification.
fn canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>, String> {
    let canonical = canonical_json_value(value);
    serde_json::to_vec(&canonical).map_err(|e| e.to_string())
}

fn canonical_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            let canonical_map: serde_json::Map<String, serde_json::Value> = sorted
                .into_iter()
                .map(|(k, v)| (k.clone(), canonical_json_value(v)))
                .collect();
            serde_json::Value::Object(canonical_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonical_json_value).collect())
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn signer_and_verifier() -> (ArtifactSigner, ArtifactVerifier) {
        let key = b"super-secret-key".to_vec();
        (ArtifactSigner::new("test-key-v1", key.clone()), ArtifactVerifier::new(key))
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let (signer, verifier) = signer_and_verifier();
        let id = Uuid::new_v4();
        let payload = serde_json::json!({"version": 1, "data": "hello"});
        let signed = signer.sign(id, payload).unwrap();
        assert!(verifier.verify(&signed).is_ok());
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let (signer, verifier) = signer_and_verifier();
        let id = Uuid::new_v4();
        let payload = serde_json::json!({"version": 1});
        let mut signed = signer.sign(id, payload).unwrap();
        signed.payload["version"] = serde_json::json!(999);
        assert!(matches!(
            verifier.verify(&signed),
            Err(SigningError::VerificationFailed { .. })
        ));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let signer = ArtifactSigner::new("k1", b"key-a".to_vec());
        let verifier = ArtifactVerifier::new(b"key-b".to_vec());
        let id = Uuid::new_v4();
        let signed = signer.sign(id, serde_json::json!({})).unwrap();
        assert!(verifier.verify(&signed).is_err());
    }

    #[test]
    fn signature_contains_key_id() {
        let (signer, _) = signer_and_verifier();
        let signed = signer.sign(Uuid::new_v4(), serde_json::json!({})).unwrap();
        assert_eq!(signed.signature.key_id, "test-key-v1");
    }

    #[test]
    fn canonical_json_different_key_order_produces_same_signature() {
        // Build two Values with the same keys in different insertion orders.
        let mut map_a = serde_json::Map::new();
        map_a.insert("z".into(), serde_json::json!(1));
        map_a.insert("a".into(), serde_json::json!(2));
        let value_a = serde_json::Value::Object(map_a);

        let mut map_b = serde_json::Map::new();
        map_b.insert("a".into(), serde_json::json!(2));
        map_b.insert("z".into(), serde_json::json!(1));
        let value_b = serde_json::Value::Object(map_b);

        let (signer, verifier) = signer_and_verifier();
        let id = Uuid::new_v4();
        let signed_a = signer.sign(id, value_a).unwrap();

        // Verify using value_b (same semantic content, different key order).
        let signed_b_check = SignedArtifact {
            artifact_id: id,
            payload: value_b,
            signature: signed_a.signature.clone(),
        };
        assert!(
            verifier.verify(&signed_b_check).is_ok(),
            "verification must succeed for semantically identical payloads regardless of key order"
        );
    }
}
