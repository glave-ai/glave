//! Ports — the trait seams. Real adapters (OpenRouter, Docker, …) implement
//! these in their own crates; in-memory fakes implement them for tests.

use anyhow::Result;

use crate::event::RunEvent;
use crate::task::TaskSpec;

/// A chat message handed to a model provider.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
        }
    }
}

/// One completion plus the accounting that makes cost-per-correct measurable.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub content: String,
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
