//! How the harness runs: the model, the prompt and instructions, what the agent may do, and the
//! limits and choices every thread starts with.

use glave_event::{Permissions, PrincipalId};
use glave_ports::settings::Judge;

use crate::commands::Command;
use crate::compaction::Compaction;
use crate::prompt::system_prompt;
use crate::retry::Retries;

/// How the harness runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The model name sent to the provider.
    pub model: String,
    /// A short name for the provider, recorded with each turn, such as `openrouter`.
    pub provider: String,
    pub system_prompt: String,
    /// The project's instructions (see [`crate::agents_md`]), sent at the start of every new
    /// thread and again after a compaction.
    pub instructions: Option<String>,
    /// What the agent may do without asking, in threads that chose no preset.
    pub permissions: Permissions,
    /// The most any thread may run with, whatever it or its project chose: the policy's cap.
    /// Full access, no cap, by default.
    pub highest: Permissions,
    /// When to summarize a thread to keep it within the model's context window.
    pub compaction: Compaction,
    /// How a model call that fails in a way that may pass is tried again.
    pub retries: Retries,
    /// Model calls allowed in one turn before it stops; no cap when `None`, as in Codex and
    /// OpenCode. A subagent's turn gets the same cap of its own.
    pub max_steps: Option<u32>,
    pub max_output_tokens: Option<u32>,
    /// Whether the model may hand work to subagents with `agent`.
    pub subagents: bool,
    /// Whether, where nothing else asks, a judge decides if an action a rule flagged may run
    /// because the person asked for it, and which model: its calls name it, for the `judge`
    /// purpose. Off by default: every flagged action asks.
    pub judge: Judge,
    /// The person whose work every thread is: recorded as the thread's `for`. When `None`,
    /// the harness makes one id when it is created, used for all its threads.
    pub person: Option<PrincipalId>,
    /// The commands the person can type as `/name arguments`: a prompt that names one is
    /// sent as its body, with the arguments put in. None by default.
    pub commands: Vec<Command>,
    /// The agent profile that runs every thread: recorded as the thread's `agent`. When
    /// `None`, made once like `person`.
    pub agent: Option<PrincipalId>,
    /// Whether a model titles each thread after its first exchange (`title.rs`); off, a
    /// thread's title is the first line of its first message. Off by default.
    pub titles: bool,
}

impl Settings {
    /// Settings for `model` at `provider`, with the default prompt naming that model.
    pub fn new(model: impl Into<String>, provider: impl Into<String>) -> Self {
        let model = model.into();
        let provider = provider.into();
        Self {
            system_prompt: system_prompt(&model, &provider),
            instructions: None,
            model,
            provider,
            permissions: Permissions::default(),
            highest: Permissions::FullAccess,
            compaction: Compaction::default(),
            retries: Retries::default(),
            max_steps: None,
            max_output_tokens: None,
            subagents: false,
            judge: Judge::Off,
            person: None,
            agent: None,
            commands: Vec::new(),
            titles: false,
        }
    }
}
