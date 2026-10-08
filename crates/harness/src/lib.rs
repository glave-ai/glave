//! glave's agent loop. A turn sends the conversation to a model, runs the tools it asks for
//! (asking the person before anything changes), and repeats until the model is done. Every
//! step is recorded as events, so the log alone can rebuild and resume any thread.

mod approval;
mod conversation;
mod env;
mod harness;
mod prompt;
mod resume;
mod sandbox;
mod secrets;
mod tools;
mod turn;

pub use approval::{AllowAll, ApprovalRequest, Approver, Decision};
pub use conversation::conversation;
pub use env::{Clock, Entropy};
pub use harness::{Harness, HarnessError, Parts, Settings, Thread};
pub use prompt::system_prompt;
pub use sandbox::{Sandbox, SandboxLevel};
pub use tools::workspace::Workspace;
pub use tools::{Tool, ToolOutcome, specs};
pub use turn::{Live, TurnSummary};
