//! glave's agent loop. A turn sends the conversation to a model, runs the tools it asks for
//! (asking the person before anything changes), and repeats until the model is done. Every
//! step is recorded as events, so the log alone can rebuild and resume any thread.

mod approval;
mod bind;
mod builtins;
mod caller;
mod calls;
mod cancel;
mod children;
mod compaction;
mod conversation;
mod env;
mod goal;
mod harness;
mod instructions;
mod join;
mod judge;
mod prompt;
mod question;
mod repeats;
mod resume;
mod retry;
mod room;
mod sandbox;
mod skills;
mod subagent;
mod todo;
mod tools;
mod toolset;
mod turn;

pub use approval::{AllowAll, ApprovalMode, ApprovalRequest, Approver, Decision, NoOneToAsk};
pub use bind::{Place, branch_for};
pub use caller::Caller;
pub use cancel::Cancel;
pub use compaction::{Compacted, Compaction};
pub use conversation::conversation;
pub use env::{Clock, Entropy, OsEntropy, SystemClock};
pub use glave_ports::secrets::{is_secret_file, is_secret_variable};
pub use glave_ports::{Sandbox, SandboxLevel};
pub use goal::{DEFAULT_BUDGET, Goal, GoalProgress, UPDATE_GOAL};
pub use harness::{Harness, HarnessError, Parts, Settings, Thread};
pub use instructions::{AGENTS_MD, Instructions, agents_md};
pub use judge::{JudgeCase, Ruling, judge};
pub use prompt::system_prompt;
pub use question::{Choice, QUESTION, Question, QuestionRequest};
pub use retry::Retries;
pub use skills::{
    Found, MAX_LISTED, MAX_SKILLS, PROJECT_FOLDERS, SKILL, SKILL_MD, Skill, Skills, find_skills,
    skill_folders,
};
pub use subagent::{AGENT, SubagentType};
pub use todo::{TODO, Todo};
pub use tools::workspace::Workspace;
pub use tools::{Tool, ToolOutcome};
pub use turn::{Live, TurnSummary};
