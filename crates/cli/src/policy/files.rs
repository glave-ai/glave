//! The two files as written: `config.toml` (choices) and `policy.toml` (constraints), every key
//! typed, and any key glave does not know refused with the file and key named.

use std::path::Path;

use anyhow::{anyhow, bail};
use glave_harness::{ApprovalMode, Hook, HookPoint, Hooks, SandboxLevel};
use globset::Glob;

use crate::policy::memory::{MemoryChoice, MemoryRules, SpaceChoice};
use crate::policy::web::WebFile;
use crate::policy::workers::{On, WorkerRules, WorkersFile};
use serde::Deserialize;

/// The only version of `policy.toml` there is.
pub const POLICY_VERSION: u32 = 1;

/// `config.toml`: what a person or project chooses. Every key is optional.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub model: Option<String>,
    /// A model, or `off`.
    pub judge_model: Option<String>,
    pub approval: Option<Approval>,
    pub sandbox: Option<SandboxChoice>,
    /// Whether sandboxed commands may use the network. A project's file may only turn it off.
    pub network: Option<bool>,
    /// The MCP servers to turn on.
    pub mcp: Option<Vec<String>>,
    /// Whether each new thread works in a worktree of its own.
    pub worktree: Option<bool>,
    /// Where new threads work: here, or on a worker of their own.
    pub on: Option<On>,
    /// The backend workers run on.
    pub workers: Option<WorkersFile>,
    /// What `web_fetch` may reach.
    pub web: Option<WebFile>,
    /// Commands to run at points of a turn. They add up across files rather than the
    /// nearest winning: the person's run, and a trusted project's run too.
    pub hooks: Option<HooksFile>,
    /// A gateway elsewhere to send model calls to, and to take the tool catalog from. Only
    /// the person's own file may set it: the person's sign-in goes there.
    pub gateway: Option<GatewayFile>,
    /// Whether glave reads and writes memory for this person. Only the person's own file sets it.
    pub memory: Option<MemoryChoice>,
    /// This space's memory: shared with the person's, isolated, or off.
    pub space_memory: Option<SpaceChoice>,
}

/// `[gateway]`: the organization's gateway (`specs/gateway.md`, "Client mode").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayFile {
    /// Where it is, such as `https://gateway.example.com`: https, or http only on this
    /// computer.
    pub url: String,
}

impl GatewayFile {
    /// Whether `url` is one glave sends a sign-in to: https, or http to this computer.
    fn safe(&self) -> bool {
        let url = self.url.trim();
        let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
            .iter()
            .any(|start| {
                url.strip_prefix(start)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
            });
        url.starts_with("https://") || local
    }
}

/// `[hooks]`: each point's hooks, under the names Claude Code and Codex give the points.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HooksFile {
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<HookEntry>,
    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<HookEntry>,
    #[serde(rename = "UserPromptSubmit", default)]
    pub user_prompt_submit: Vec<HookEntry>,
    #[serde(rename = "Stop", default)]
    pub stop: Vec<HookEntry>,
}

/// One hook as written: `[[hooks.PreToolUse]]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookEntry {
    pub command: String,
    /// A regular expression over the whole tool name; every tool when absent.
    pub matcher: Option<String>,
    /// Seconds before it is stopped.
    pub timeout: Option<u64>,
}

impl HooksFile {
    /// Every hook, point by point, in the order written.
    pub fn hooks(&self) -> Vec<Hook> {
        let points = [
            (HookPoint::PreToolUse, &self.pre_tool_use),
            (HookPoint::PostToolUse, &self.post_tool_use),
            (HookPoint::UserPromptSubmit, &self.user_prompt_submit),
            (HookPoint::Stop, &self.stop),
        ];
        points
            .into_iter()
            .flat_map(|(point, entries)| {
                entries.iter().map(move |entry| Hook {
                    point,
                    command: entry.command.clone(),
                    matcher: entry.matcher.clone(),
                    timeout_secs: entry.timeout,
                })
            })
            .collect()
    }
}

/// An approval mode as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Approval {
    Auto,
    Ask,
}

impl From<ApprovalMode> for Approval {
    fn from(mode: ApprovalMode) -> Self {
        match mode {
            ApprovalMode::Auto => Self::Auto,
            ApprovalMode::Ask => Self::Ask,
        }
    }
}

impl Approval {
    pub fn mode(self) -> ApprovalMode {
        match self {
            Self::Auto => ApprovalMode::Auto,
            Self::Ask => ApprovalMode::Ask,
        }
    }
}

/// A sandbox level as written, `off` included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxChoice {
    Off,
    ReadOnly,
    Project,
}

impl SandboxChoice {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "off" => Some(Self::Off),
            "read-only" => Some(Self::ReadOnly),
            "project" => Some(Self::Project),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::ReadOnly => "read-only",
            Self::Project => "project",
        }
    }

    pub fn level(self) -> Option<SandboxLevel> {
        match self {
            Self::Off => None,
            Self::ReadOnly => Some(SandboxLevel::ReadOnly),
            Self::Project => Some(SandboxLevel::Project),
        }
    }
}

/// `policy.toml`: what an organization allows. Every section is optional; the version is not.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyFile {
    pub version: u32,
    #[serde(default)]
    pub models: ModelRules,
    #[serde(default)]
    pub approval: ApprovalRules,
    #[serde(default)]
    pub sandbox: SandboxRules,
    #[serde(default)]
    pub mcp: McpRules,
    #[serde(default)]
    pub budget: BudgetRules,
    #[serde(default)]
    pub projects: ProjectRules,
    #[serde(default)]
    pub hooks: HookRules,
    #[serde(default)]
    pub workers: WorkerRules,
    #[serde(default)]
    pub tasks: TaskRules,
    #[serde(default)]
    pub memory: MemoryRules,
}

/// `[tasks]`: what one person may run with no one attached, and keep scheduled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRules {
    /// Tasks one person may have running at once.
    pub max_running: Option<usize>,
    /// Schedules one person may keep.
    pub max_schedules: Option<usize>,
}

/// `[hooks]`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookRules {
    /// Whether people and projects may run hooks of their own.
    pub own: Option<bool>,
}

/// `[projects]`: which projects need no question.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRules {
    /// Globs over a repository's canonical path: trusted without asking.
    #[serde(default)]
    pub trusted: Vec<String>,
}

/// `[models]`: globs over `provider/model`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRules {
    pub allow: Option<Vec<String>>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// `[approval]`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRules {
    pub modes: Option<Vec<Approval>>,
    pub judge: Option<JudgeRule>,
}

/// Whether the judge must, may or must not decide flagged actions in auto mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JudgeRule {
    Required,
    Allowed,
    Off,
}

impl JudgeRule {
    pub fn name(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Allowed => "allowed",
            Self::Off => "off",
        }
    }
}

/// `[sandbox]`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxRules {
    #[serde(default)]
    pub required: bool,
}

/// `[mcp]`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpRules {
    /// The servers the organization offers.
    pub catalog: Option<Vec<String>>,
    /// Whether people may add servers of their own.
    pub own: Option<bool>,
    /// Servers whose tools return untrusted content, as web pages are.
    #[serde(default)]
    pub untrusted: Vec<String>,
}

/// `[budget]`.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetRules {
    pub person_per_day_usd: Option<f64>,
}

/// `config.toml` at `path`, from its `text`.
pub fn config(text: &str, path: &Path) -> anyhow::Result<ConfigFile> {
    let file: ConfigFile =
        toml::from_str(text).map_err(|error| anyhow!("{}: {error}", path.display()))?;
    if let Some(hooks) = &file.hooks {
        Hooks::new(hooks.hooks()).map_err(|error| anyhow!("{}: {error}", path.display()))?;
    }
    if let Some(gateway) = &file.gateway
        && !gateway.safe()
    {
        bail!(
            "{}: [gateway] url = {:?} is not https (http only to this computer): your sign-in \
             goes there",
            path.display(),
            gateway.url
        );
    }
    if let Some(web) = &file.web {
        web.policy()
            .map_err(|error| anyhow!("{}: {error:#}", path.display()))?;
    }
    Ok(file)
}

/// `policy.toml` at `path`, from its `text`: a version glave knows, and globs that compile.
pub fn policy(text: &str, path: &Path) -> anyhow::Result<PolicyFile> {
    let file: PolicyFile =
        toml::from_str(text).map_err(|error| anyhow!("{}: {error}", path.display()))?;
    if file.version != POLICY_VERSION {
        bail!(
            "{}: version = {} is not one this glave knows: use version = {POLICY_VERSION}",
            path.display(),
            file.version
        );
    }
    let globs = file
        .models
        .allow
        .iter()
        .flatten()
        .map(|glob| ("allow", glob));
    let denied = file.models.deny.iter().map(|glob| ("deny", glob));
    for (key, glob) in globs.chain(denied) {
        if let Err(error) = Glob::new(glob) {
            bail!(
                "{}: [models] {key} has {glob:?}, which is not a glob: {error}",
                path.display()
            );
        }
    }
    for glob in &file.projects.trusted {
        if let Err(error) = Glob::new(glob) {
            bail!(
                "{}: [projects] trusted has {glob:?}, which is not a glob: {error}",
                path.display()
            );
        }
    }
    if let Some(usd) = file.budget.person_per_day_usd
        && !(usd.is_finite() && usd >= 0.0)
    {
        bail!(
            "{}: [budget] person_per_day_usd = {usd} is not an amount of dollars",
            path.display()
        );
    }
    Ok(file)
}
