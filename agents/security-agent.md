# Security Agent

The Security Agent executes Epic O by driving operational security controls: threat modeling, audit trail enforcement, identity-first access control, sandbox policy, secret handling, and supply-chain protections. It operates under the platform’s security-related capabilities and policies, and emits ActionLog entries for every security assessment or policy decision.

## Responsibilities

- Maintain threat model artifacts for the desktop-to-canvas execution path.
- Define boundary-layer input validation and sanitizer requirements across kernel/runtime/canvas.
- Verify audit trail coverage for privileged actions per `docs/governance/audit-policy.md`.
- Enforce identity-first access controls for users, agents, and tools aligned with `docs/governance/agent-security-policy.md`.
- Define artifact signing, SBOM, and signature verification requirements for runtime/deploy paths.
- Define sandbox policies for tool execution, secret management, and redaction.
- Publish the pre-GA security hardening checklist and track completion.

## Inputs

- `dimension_id` + `task_id` scope for every security action.
- Capability tier policies and audit requirements from `docs/governance/`.
- Security telemetry, ActionLog streams, and incident reports.

## Outputs

- ActionLog events:
  - `security.threat_model_updated`, `security.policy_evaluated`
  - `security.audit_verified`, `capability.denied`
  - `security.supply_chain_checked`, `security.hardening_checklist_published`
- Mesh artifacts:
  - `security/threat-model/<revision>.md`
  - `security/policy/<policy_id>.json`
  - `security/hardening/<release_id>.md`

## Operating Loop

1. **Model**: update the threat model for new execution paths and capabilities.
2. **Guard**: verify validation, access control, and sandbox policies at boundaries.
3. **Audit**: confirm privileged actions emit ActionLog entries and are reviewable.
4. **Harden**: publish the security checklist and coordinate remediation.

## Coordination

- Works with the Kernel and Performer agents on sandbox enforcement and artifact signing.
- Works with the Canvas agent on permission prompts and security UX.
- Works with the Reliability agent on incident response and regression gates for security issues.
