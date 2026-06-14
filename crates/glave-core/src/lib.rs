//! glave-core — the brain.
//!
//! Domain types, ports (traits), and application logic (the agent loop, the tool
//! grammar, mediation policy, orchestration, oracle scoring). Depends only on its
//! ports — never on a concrete model provider or execution backend — so both the
//! client/server moat boundary and full testability are enforced by the compiler.

pub mod agent;
pub mod capability;
pub mod engine;
pub mod event;
pub mod hostfs;
pub mod local;
pub mod mcp;
pub mod oracle;
pub mod policy;
pub mod ports;
pub mod run;
pub mod runtime;
pub mod sinks;
pub mod task;
pub mod tools;

#[cfg(any(test, feature = "test-util"))]
pub mod testkit;

pub use capability::{
    AgentDef, Capability, CapabilityKind, CapabilityRegistry, LexicalSelector, Selector,
};
pub use engine::{AgentSpec, RunEngine, RunOutcome, RunRequest};
pub use event::RunEvent;
pub use local::LocalRuntime;
pub use runtime::{open_runtime, Location};
pub use mcp::{McpClient, McpHub, McpServerConfig, McpTool, McpTransport, StdioTransport};
pub use oracle::Verdict;
pub use policy::{AutonomousPolicy, Decision, Policy};
pub use ports::{
    Completion, EvalRuntime, ExecResult, Grader, Message, ModelProvider, ProviderFactory, Role,
    RuntimeFactory, ToolCallRequest, TraceSink, Workspace,
};
pub use sinks::{InMemorySink, JsonlSink, NullSink};
pub use task::TaskSpec;
pub use tools::{tool_specs, ToolCall, ToolSpec};
