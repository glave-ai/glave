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

/// Port: a swappable execution substrate (Docker now; Firecracker / remote later).
pub trait WorkspaceRuntime {
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

/// Port: prepares a fresh, isolated `WorkspaceRuntime` for one run. The returned
/// runtime cleans up its own resources on drop.
pub trait RuntimeFactory: Send + Sync {
    fn prepare(&self, task: &TaskSpec, run_id: &str) -> Result<Box<dyn WorkspaceRuntime>>;
}
