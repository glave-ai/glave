//! `policy.toml` as written: what an organization allows, every key typed.

use std::path::Path;

use glave_event::Permissions;
use globset::Glob;
use serde::Deserialize;

use crate::error::invalid;
use crate::policy::memory::MemoryRules;
use crate::policy::unreadable::unreadable;
use crate::policy::workers::WorkerRules;

/// The only version of `policy.toml` there is.
pub(crate) const POLICY_VERSION: u32 = 1;

/// `policy.toml`: what an organization allows. Every section is optional; the version is not.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyFile {
    pub version: u32,
    #[serde(default)]
    pub models: ModelRules,
    #[serde(default)]
    pub permissions: PermissionRules,
    #[serde(default)]
    pub approval: ApprovalRules,
    #[serde(default)]
    pub sandbox: SandboxRules,
    #[serde(default)]
    pub mcp: McpRules,
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
    #[serde(default)]
    pub notifications: NotificationRules,
}

/// `[notifications]`: what reaches people's devices from the server.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationRules {
    /// Whether the server pushes notifications to people's browsers (Web Push).
    pub push: Option<bool>,
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

/// `[permissions]`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRules {
    /// The most anyone may choose: below `full-access`, commands always run in the sandbox.
    pub highest: Option<Permissions>,
}

/// `[approval]`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRules {
    pub judge: Option<JudgeRule>,
}

/// Whether the judge must, may or must not decide flagged actions where nothing else asks.
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
    /// Whether commands may use the network: `false` takes it from them, whatever anyone
    /// chooses.
    pub network: Option<bool>,
    /// Whether a command without the network may ask the person to run with it: `false`
    /// forbids asking.
    pub ask_network: Option<bool>,
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
    /// Servers vouched for: remote ones and a project's, untrusted by default, are trusted.
    #[serde(default)]
    pub trusted: Vec<String>,
}

/// `policy.toml` at `path`, from its `text`: a version glave knows, and globs that compile.
pub(crate) fn policy(text: &str, path: &Path) -> crate::Result<PolicyFile> {
    let file: PolicyFile = toml::from_str(text).map_err(|error| unreadable(path, text, &error))?;
    if file.version != POLICY_VERSION {
        invalid!(
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
            invalid!(
                "{}: [models] {key} has {glob:?}, which is not a glob: {error}",
                path.display()
            );
        }
    }
    for glob in &file.projects.trusted {
        if let Err(error) = Glob::new(glob) {
            invalid!(
                "{}: [projects] trusted has {glob:?}, which is not a glob: {error}",
                path.display()
            );
        }
    }
    Ok(file)
}
