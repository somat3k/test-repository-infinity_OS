# Security Hardening Checklist — Before GA

**Status:** `[x]` complete (checklist created; all items tracked)  
**Epic:** O — Operational Security (item 10)  
**Owner:** security-agent  
**Date:** 2026-03-11

---

## Purpose

This checklist must be completed and signed off by the security-agent and a designated approver before any General Availability (GA) release of infinityOS.  Each item links to the implementing module or governance document.

---

## 1. Identity and Access Control

- [x] `IdentityRegistry` registers all principals (users, agents, tools) before any access is permitted.
- [x] `AccessPolicy::check` is called for every resource access; `CapabilityDenied` is never swallowed.
- [x] No component holds `Capabilities::ADMIN` unless explicitly granted by operator policy.
- [x] Agent templates declare `required_capabilities` and the runtime verifies them before instantiation.
- [ ] **GA gate**: Production identity provider (SSO/JWT) integrated; `IdentityRegistry` backed by persistent store.
- [ ] **GA gate**: Capability audit run — verify no principal holds capabilities beyond minimum required.

---

## 2. Input Validation

- [x] `InputValidator` rules registered for all six `BoundaryLayer` variants.
- [x] Validation runs before data crosses any layer boundary (canvas→runtime, runtime→kernel, mesh write, tool invocation, agent input, API ingress).
- [x] `SecurityValidationFailed` ActionLog event emitted on rejection.
- [ ] **GA gate**: Fuzz-test all `ValidationRule` implementations with corpus of malformed inputs (see Epic Q).
- [ ] **GA gate**: Confirm no boundary accepts raw user input without passing through `InputValidator`.

---

## 3. Audit Trail

- [x] `PrivilegedAuditLog::record` called for every action requiring `CAP_DEPLOY`, `CAP_READ_SECRETS`, `CAP_ADMIN`, `CAP_PUBLISH_MARKETPLACE`.
- [x] Hash-chain integrity verifiable via `PrivilegedAuditLog::verify_chain`.
- [x] Audit records forwarded to ActionLog for mesh persistence and telemetry.
- [ ] **GA gate**: Audit records retained for ≥ 12 months (per `docs/governance/audit-policy.md`).
- [ ] **GA gate**: Audit bundle export path tested for compliance review.
- [ ] **GA gate**: Replace FNV hash with SHA-256 HMAC for tamper-evidence strength.

---

## 4. Artifact Signing

- [x] All mesh artifacts and deployment payloads signed via `ArtifactSigner` before publication.
- [x] All consumers call `ArtifactVerifier::verify` before processing an artifact.
- [x] `SecurityArtifactVerified` / `SecurityArtifactVerificationFailed` ActionLog events emitted.
- [ ] **GA gate**: Replace FNV-based signing with Ed25519 (`ed25519-dalek` crate).
- [ ] **GA gate**: Key rotation procedure documented and tested.
- [ ] **GA gate**: Artifact signing keys stored in HSM or OS keychain, never in source.

---

## 5. Sandboxed Tool Execution

- [x] Every tool has a `SandboxProfile` registered in `SandboxPolicy`.
- [x] `SandboxEnforcer::check` called before any tool filesystem, network, or model access.
- [x] `SecuritySandboxViolation` ActionLog event emitted on denial.
- [ ] **GA gate**: Sandbox escape tests pass (see Epic Q security SAST/DAST pipeline).
- [ ] **GA gate**: OS-level sandboxing (e.g., seccomp, namespaces) applied to tool subprocess execution.
- [ ] **GA gate**: `allow_fs_write` and `allow_network` default to `false`; explicit grant required.

---

## 6. Secret Management

- [x] All secrets registered in `SecretStore`; never stored in plain config files or environment variables.
- [x] `Redactor` patterns registered for all known secret values at startup.
- [x] `Redactor::redact` / `redact_json` applied to all canvas node output and artifact payloads before render or storage.
- [ ] **GA gate**: `SecretStore` backed by OS keychain or external KMS (Vault, AWS Secrets Manager).
- [ ] **GA gate**: Secret rotation procedure documented; old patterns purged from `Redactor` after rotation.
- [ ] **GA gate**: No secret value appears in telemetry, logs, or UI output (verified by automated scan).

---

## 7. Supply Chain

- [x] SBOM generated at build time with all direct and transitive dependencies.
- [x] `SupplyChainVerifier::verify_sbom` run before installation of any component.
- [x] `SecuritySupplyChainVerified` ActionLog event emitted after successful verification.
- [ ] **GA gate**: Replace FNV-based component signing with Ed25519.
- [ ] **GA gate**: SBOM published alongside every release artifact.
- [ ] **GA gate**: `Sbom::vulnerable_components()` checked in CI; build fails on any known CVE with CVSS ≥ 7.0.
- [ ] **GA gate**: Dependency pinning enforced (`Cargo.lock` checked in; lock file integrity verified).

---

## 8. Policy Engine

- [x] `PolicyEngine` configured with rules covering all principal kinds, action types, and resource kinds.
- [x] Default-deny posture: unmatched requests return `Decision::Deny`.
- [x] `SecurityAccessDenied` ActionLog event emitted for every denied request.
- [ ] **GA gate**: Policy rules reviewed and approved by security-agent before GA.
- [ ] **GA gate**: Rate-limit rules enforced per dimension to prevent task queue exhaustion.
- [ ] **GA gate**: Policy rule changes require a signed ActionLog entry (`PrivilegedAdminChange`).

---

## 9. Threat Model Review

- [x] Desktop-to-canvas threat model documented in `docs/architecture/security-threat-model.md`.
- [x] All Critical and High inherent-risk threats have at least one implemented mitigation.
- [x] Residual risk ≤ inherent risk for all threat entries.
- [ ] **GA gate**: Threat model reviewed by an independent security reviewer.
- [ ] **GA gate**: T-07 residual risk reduced from Medium to Low after Ed25519 signing is implemented.

---

## 10. General Hardening

- [ ] **GA gate**: SAST scan (CodeQL or equivalent) passes with zero High/Critical findings.
- [ ] **GA gate**: DAST scan on API ingress endpoints passes.
- [ ] **GA gate**: `#![forbid(unsafe_code)]` enforced across all Rust crates (already enforced in this crate).
- [ ] **GA gate**: Third-party security audit completed.
- [ ] **GA gate**: Incident response plan reviewed and exercised (see `docs/governance/incident-response.md`).
- [ ] **GA gate**: Release gate checklist signed off by security-agent and a designated engineering approver.

---

## Sign-off

| Role | Name | Date | Signature |
|------|------|------|-----------|
| Security Agent | TBD | — | — |
| Engineering Approver | TBD | — | — |

---

## References

- `runtime-rust/crates/ify-security/` — Epic O implementation
- `docs/architecture/security-threat-model.md` — threat model
- `docs/governance/audit-policy.md` — audit retention
- `docs/governance/agent-security-policy.md` — agent security policy
- `docs/governance/release-gates.md` — release gate process
- `docs/governance/incident-response.md` — incident response
