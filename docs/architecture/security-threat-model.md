# Security Threat Model — Desktop-to-Canvas Execution Path

**Status:** `[x]` complete  
**Epic:** O — Operational Security  
**Owner:** security-agent  
**Date:** 2026-03-11

---

## 1. Scope

This document covers the threat model for the **desktop-to-canvas execution path** in infinityOS: from a user action on the desktop shell through the Performer Runtime, Kernel FFI, and finally to the Mesh Artifact Bus and Canvas UI.

The typed threat model is implemented in `runtime-rust/crates/ify-security/src/threat_model.rs` and can be queried programmatically.

---

## 2. System Context

```
User (Desktop)
    │
    ▼  [canvas boundary]
Canvas UI  ──────────────────────────────► Mesh Artifact Bus
    │                                           ▲
    │  [canvas→runtime boundary]                │
    ▼                                           │
Performer Runtime (Orchestrator)  ─────────────┘
    │  [runtime→kernel FFI boundary]
    ▼
Kernel (C) / FFI
    │
    ▼
OS / Hardware
```

---

## 3. Threat Catalogue (STRIDE)

All threats are encoded in `ThreatModel::desktop_to_canvas()`.

| ID   | Title | STRIDE Category | Layer | Inherent Risk | Residual Risk | Mitigation |
|------|-------|-----------------|-------|---------------|---------------|------------|
| T-01 | Agent impersonation via forged TaskID | Spoofing | Agent | High | Low | Identity verification at orchestrator boundary |
| T-02 | Artifact tampering via unsigned mesh write | Tampering | Mesh | High | Low | Artifact signing (see `artifact_signing` module) |
| T-03 | Privileged action repudiation | Repudiation | Runtime | Medium | Low | Hash-chained audit trail (see `audit` module) |
| T-04 | Secret leakage through canvas node output | Information Disclosure | Canvas | Critical | Low | Redactor intercepts all string outputs |
| T-05 | Sandbox escape via unrestricted filesystem access | Elevation of Privilege | Runtime | Critical | Low | SandboxPolicy path prefix enforcement |
| T-06 | Capability escalation through unvalidated boundary input | Elevation of Privilege | Kernel | High | Low | InputValidator at all layer boundaries |
| T-07 | Supply chain compromise via unsigned dependency | Tampering | Desktop | High | Medium | SBOM + SupplyChainVerifier |
| T-08 | Denial of service via unbounded task queue | Denial of Service | Runtime | Medium | Low | PolicyEngine rate limiting |

---

## 4. Mitigations Summary

### 4.1 Identity Verification (T-01)
- `ify-security::identity`: `Principal`, `IdentityRegistry`, `AccessPolicy`.
- Every resource access checked against `ResourceKind::required_capability()`.
- Access denials are logged via `tracing::warn!`.

### 4.2 Artifact Signing (T-02)
- `ify-security::artifact_signing`: `ArtifactSigner`, `ArtifactVerifier`, `SignedArtifact`.
- All mesh and deploy artifacts signed before publication.
- Consumers call `ArtifactVerifier::verify` before processing.
- Payload is serialised using canonical JSON (sorted keys at every nesting level) so that semantically identical values always produce the same signature regardless of map-key insertion order.
- ActionLog integration at call sites is deferred; see `docs/governance/security-hardening-checklist.md` GA gate §4.

### 4.3 Privileged Action Audit (T-03)
- `ify-security::audit`: `PrivilegedAuditLog`, `AuditRecord`.
- `record()` validates that `caps` contains `kind.required_capability()`; returns `AuditError::MissingCapabilityContext` otherwise.
- `record()` accepts an `Actor` value; the actor kind is preserved in the stored record.
- Hash-chained records stored with causality and correlation IDs; `verify_chain()` recomputes each record's hash from its fields to detect tampering and verifies chain linkage separately.
- Minimum 12-month retention per `docs/governance/audit-policy.md`.

### 4.4 Secret Redaction (T-04)
- `ify-security::secrets`: `SecretStore`, `Redactor`.
- `Redactor::redact` / `redact_json` called at all canvas output paths.
- Patterns registered once at startup; all outputs scrubbed before rendering.

### 4.5 Sandbox Enforcement (T-05)
- `ify-security::sandbox`: `SandboxPolicy`, `SandboxProfile`, `SandboxEnforcer`.
- Each tool declares allowed paths, hosts, and model IDs.
- Path checks use `std::path::Path::starts_with` (component-boundary semantics), preventing bypass via prefix extension (e.g. `/tmp/workdir2` does NOT match prefix `/tmp/workdir`).
- `PathAccess::Write` accesses additionally require `allow_fs_write` in the profile; returns `SandboxError::WriteNotAllowed` otherwise.
- `SandboxEnforcer::check` called before every tool invocation.
- When ActionLog is attached via `SandboxEnforcer::with_action_log`, `SecuritySandboxViolation` is emitted on every denial.

### 4.6 Input Validation (T-06)
- `ify-security::validator`: `InputValidator`, `BoundaryLayer`.
- Rules registered per layer (canvas→runtime, runtime→kernel, mesh write, tool invocation, agent input, API ingress).
- Validation runs before data crosses any layer boundary.
- When ActionLog is attached via `InputValidator::with_action_log`, `SecurityValidationFailed` is emitted on every failure.

### 4.7 Supply Chain (T-07)
- `ify-security::supply_chain`: `Sbom`, `ComponentRecord`, `SupplyChainVerifier`.
- SBOM generated at build time; published alongside release artifacts.
- `SupplyChainVerifier::verify_sbom` run before installation.
- ActionLog integration at call sites is deferred; see `docs/governance/security-hardening-checklist.md` GA gate §7.
- Residual risk remains **Medium** until asymmetric signing (Ed25519) replaces FNV mixing.

### 4.8 Policy Engine (T-08)
- `ify-security::policy`: `PolicyEngine`, `PolicyRule`, `Decision`.
- Default-deny posture: no rule → Deny.
- Rate-limit and quota rules registered per dimension.
- When ActionLog is attached via `PolicyEngine::with_action_log`, `SecurityAccessDenied` is emitted for every deny outcome (explicit deny rule or default-deny).

---

## 5. Residual Risk Acceptance

| ID   | Residual | Accepted By | Notes |
|------|----------|-------------|-------|
| T-07 | Medium | Security team | Upgrade to Ed25519 before GA. Tracked in Epic O item 5. |

All other threats: residual risk **Low** — accepted.

---

## 6. References

- `runtime-rust/crates/ify-security/src/` — implementation
- `docs/governance/security-hardening-checklist.md` — GA readiness checklist
- `docs/governance/audit-policy.md` — audit retention requirements
- `docs/architecture/capability-registry.md` — capability taxonomy
- `docs/architecture/event-taxonomy.md` — ActionLog event types §3.8
