//! glave-core — the brain.
//!
//! Domain types, ports (traits), and application logic (the agent loop,
//! orchestration, oracle scoring). Depends only on its ports — never on a
//! concrete model provider or execution backend — so both the client/server
//! moat boundary and full testability are enforced by the compiler.

pub mod agent;
pub mod event;
pub mod oracle;
pub mod ports;
pub mod run;
pub mod sinks;
pub mod task;

#[cfg(any(test, feature = "test-util"))]
pub mod testkit;

pub use event::RunEvent;
pub use oracle::Verdict;
pub use ports::{Completion, ExecResult, Message, ModelProvider, TraceSink, WorkspaceRuntime};
pub use task::TaskSpec;
