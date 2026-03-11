# agents — Built-in Agent Templates and Policies

The agents layer owns built-in agent template definitions, execution policies, marketplace foundations, and the agent execution flows that run on top of the Performer Runtime.

## Responsibilities

- Agent template definitions (capabilities, tools, memory wiring, task-flow)
- Execution policy enforcement (allowed capabilities by tier)
- Marketplace package format and registry (local + remote)
- Signature verification and trust policy for marketplace content
- Snippet-to-node compiler (turn editor code into reusable node templates)
- Sandboxing requirements for marketplace content
- Rollback/disable mechanisms for compromised packages

## Constraints

- **Depends on Canvas and Performer Runtime**: no direct calls to data, deploy, or kernel layers.
- **Policy-enforced execution**: every agent action must pass through the capability registry before execution.
- **Signed packages**: all marketplace submissions require signing before publish.
- **Least-privilege default**: agents request only the capabilities they declare; no ambient authority.

## Built-in Templates

- [Performance Optimization Agent](performance-optimization-agent.md)
- [Reliability Agent](reliability-agent.md)
- [Security Agent](security-agent.md)

## Epic Tracking

See [EPIC O — Operational Security](../TODO.md) and [EPIC S — Snippet and Agent Marketplace Foundations](../TODO.md) in `TODO.md`.
