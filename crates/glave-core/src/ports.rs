//! Ports — the trait seams. Real adapters (OpenRouter, Docker, …) implement
//! these in their own crates; in-memory fakes implement them for tests.

use anyhow::Result;

use crate::event::RunEvent;
use crate::task::TaskSpec;

/// A structured tool call the model requested (native function-calling): a
/// stable `id`, the tool `name`, and raw JSON `arguments` as the provider returns
/// them. The id correlates the call with its `tool`-role result message.
#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// The author of a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// A chat message handed to a model provider. Carries the native tool-calling
/// envelope: an assistant turn may include `tool_calls`; a `tool`-role result
/// names the `tool_call_id` it answers.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain(Role::System, content)
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(Role::User, content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(Role::Assistant, content)
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// An assistant turn that issued tool calls (content may be empty).
    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRequest>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// A `tool`-role result answering a specific tool call by id.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// One completion plus the accounting that makes cost-per-correct measurable.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub content: String,
    /// The model's reasoning/thinking text this turn, when the server surfaces it
    /// (OpenRouter `reasoning`, vLLM `reasoning_content`). Kept separate from
    /// `content` so it's captured uniformly regardless of how each server frames
    /// it on the wire — the core never sees the per-provider shape.
    pub reasoning: Option<String>,
    /// Structured tool calls the model made this turn (native tool-calling).
    pub tool_calls: Vec<ToolCallRequest>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Reasoning tokens (subset of completion) — the cost lever, tracked alone.
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
    pub model: String,
    pub provider: Option<String>,
}

/// Port: a swappable model backend (OpenRouter now; vLLM / our own inference later).
pub trait ModelProvider {
    fn complete(&self, messages: &[Message]) -> Result<Completion>;
    fn model_id(&self) -> &str;
}

/// Result of running a command in the sandbox.
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Port: the workspace the agent acts in — read, edit, run, search. This is the
/// WHOLE surface the agent ever touches. It deliberately has **no `grade`**: the
/// held oracle tests are unreachable from here, so the oracle-seal is a compiler
/// guarantee rather than a convention. (Docker now; Firecracker / remote later.)
///
/// The seal, enforced as a compile-fail doctest — grading through the agent's
/// handle must NOT compile. If anyone re-adds `grade` to `Workspace` (re-exposing
/// the held tests to the agent), this starts compiling and the test fails:
/// ```compile_fail
/// # use glave_core::Workspace;
/// fn tries_to_grade(w: &dyn Workspace) {
///     w.grade("pytest"); // no such method on `Workspace` — that is the seal
/// }
/// ```
pub trait Workspace {
    fn read_file(&self, rel: &str) -> Result<String>;
    fn write_file(&mut self, rel: &str, contents: &str) -> Result<()>;
    fn list_files(&self) -> Result<Vec<String>>;
    /// Run a shell command in the workspace dir inside the sandbox.
    fn run(&self, cmd: &str) -> Result<ExecResult>;
    /// Search file contents (git-aware) — implemented natively by each runtime
    /// where the files live, so we never depend on a sandbox `rg`. Returns
    /// `path:line:text` lines; `file_glob` optionally restricts which files.
    fn grep(&self, pattern: &str, file_glob: Option<&str>) -> Result<Vec<String>>;
    /// Find files by glob (git-aware). Returns matching workspace-relative paths.
    fn find(&self, glob: &str) -> Result<Vec<String>>;
}

/// Port: ORACLE execution at grade time — **eval-only, never handed to the
/// agent**. Runs the held tests outside the agent's sandbox (a separate
/// ephemeral container with the oracle mounted) so they stay sealed until
/// grading. Kept off [`Workspace`] on purpose: the agent has no `Grader`, so it
/// cannot read or invoke the grader — the compiler enforces the seal.
pub trait Grader {
    fn grade(&self, cmd: &str) -> Result<ExecResult>;
}

/// A runtime usable for an EVAL run: both the agent's [`Workspace`] AND the
/// [`Grader`]. The eval path needs both from one object; the product path needs
/// only `Workspace`. Blanket-implemented, so any type that is both is an
/// `EvalRuntime` automatically. Trait upcasting then lets one
/// `&mut dyn EvalRuntime` hand the agent a `&mut dyn Workspace` and the oracle a
/// `&dyn Grader` — without the agent ever gaining `grade`.
pub trait EvalRuntime: Workspace + Grader {}
impl<T: Workspace + Grader> EvalRuntime for T {}

/// Port: a swappable trace destination (JSONL file now; DB / remote later).
pub trait TraceSink {
    fn emit(&mut self, event: RunEvent) -> Result<()>;
}

/// Port: mints a `ModelProvider` for a given model id. Per-call / per-agent
/// model selection, routing, and fallback live behind this — the gateway is its
/// real implementation. `Send + Sync` so one factory serves every concurrent run.
pub trait ProviderFactory: Send + Sync {
    fn create(&self, model: &str) -> Result<Box<dyn ModelProvider>>;
}

/// Port: prepares a fresh, isolated [`EvalRuntime`] for one eval run (workspace +
/// grader). The returned runtime cleans up its own resources on drop.
pub trait RuntimeFactory: Send + Sync {
    fn prepare(&self, task: &TaskSpec, run_id: &str) -> Result<Box<dyn EvalRuntime>>;
}

#[cfg(test)]
mod seal {
    //! Oracle seal, locked at compile time. The agent only ever receives a
    //! [`Workspace`], which has no `grade`; grading is reachable solely through
    //! [`Grader`], a capability the agent is never handed. If someone moved
    //! `grade` onto `Workspace` (re-exposing the held tests to the agent), the
    //! intent encoded below would be broken — keep grading on `Grader` only.
    use super::*;

    #[allow(dead_code)]
    fn agent_sees_only_workspace(_w: &mut dyn Workspace) {
        // `_w.grade(...)` intentionally does not exist — that is the seal.
    }

    #[allow(dead_code)]
    fn grading_requires_the_grader(g: &dyn Grader) -> Result<ExecResult> {
        g.grade("true")
    }
}
