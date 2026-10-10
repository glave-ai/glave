//! The two files as written: `config.toml` (choices) and `policy.toml` (constraints), every key
//! typed, and any key glave does not know refused with the file and key named.

use std::path::Path;

use anyhow::{anyhow, bail};
use glave_harness::{ApprovalMode, SandboxLevel};
use globset::Glob;
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
    /// The MCP servers to turn on.
    pub mcp: Option<Vec<String>>,
    /// Whether each new thread works in a worktree of its own.
    pub worktree: Option<bool>,
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
}

/// `[budget]`.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetRules {
    pub person_per_day_usd: Option<f64>,
}

/// `config.toml` at `path`, from its `text`.
pub fn config(text: &str, path: &Path) -> anyhow::Result<ConfigFile> {
    toml::from_str(text).map_err(|error| anyhow!("{}: {error}", path.display()))
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
