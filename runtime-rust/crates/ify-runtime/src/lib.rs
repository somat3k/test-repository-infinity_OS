//! # ify-runtime — infinityOS Rust Performer Runtime
//!
//! This is the top-level runtime crate for the infinityOS Performer layer.
//! It orchestrates the full agentic execution stack and exposes:
//!
//! - [`agent_executor`] — executor for agentic combo ML tasks.
//! - [`tool_runner`]    — typed abstraction over db/http/blockchain/model tools.
//! - [`memory`]         — short-term, long-term, and vector-store memory subsystem.
//! - [`planner`]        — plan → tasks → node-graph integration.
//! - [`yield_token`]    — cooperative cancellation and yield primitives.
//! - [`sandbox`]        — capability-gated sandbox integration.
//! - [`telemetry`]      — structured logging and OpenTelemetry trace init.
//!
//! ## Architecture Position
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                       ify-runtime                            │
//! │  AgentExecutor ──► ToolRunner ──► MemorySubsystem            │
//! │        │               │                                     │
//! │        ▼               ▼                                     │
//! │     Planner        Sandbox ◄── capabilities from kernel      │
//! │        │                                                     │
//! │        ▼                                                     │
//! │   ify-executor (task lifecycle)                              │
//! │   ify-ffi      (kernel calls)                                │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Invariants
//!
//! - All public APIs return `Result<T, E>` — panics are forbidden in library code.
//! - Every runtime action that touches execution or memory must carry a
//!   `TaskId` and `DimensionId` for ActionLog compatibility.
//! - Capabilities are checked at sandbox entry; callers receive
//!   [`sandbox::SandboxError::CapabilityDenied`] rather than silent failures.

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod agent_executor;
pub mod memory;
pub mod planner;
pub mod sandbox;
pub mod telemetry;
pub mod tool_runner;
pub mod yield_token;

// Re-export the most-used top-level types for convenience.
pub use agent_executor::{AgentExecutor, AgentExecutorConfig, AgentTask, AgentTaskKind};
pub use memory::{MemorySubsystem, MemorySubsystemConfig};
pub use planner::{Plan, PlanStep, Planner, PlannerResult};
pub use sandbox::{Sandbox, SandboxError, SandboxPolicy};
pub use telemetry::{TelemetryConfig, TelemetryHandle};
pub use tool_runner::{ToolKind, ToolRegistry, ToolRequest, ToolResponse, ToolRunner};
pub use yield_token::YieldToken;
