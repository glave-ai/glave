//! The two files as written: `config.toml` (choices) and `policy.toml` (constraints), every key
//! typed, and any key glave does not know refused with the file and key named.

use std::path::Path;

use crate::error::invalid;
use glave_harness::{ApprovalMode, Hook, HookPoint, Hooks, SandboxLevel};
use glave_ports::auth::safe_to_send;
use glave_ports::{CredentialHost, Forge};
use globset::Glob;

use crate::policy::memory::{MemoryChoice, MemoryRules, SpaceChoice};
use crate::policy::placement::PlacementRules;
use crate::policy::provider::ProviderFile;
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
    pub approval: Option<ApprovalMode>,
    pub sandbox: Option<SandboxChoice>,
    /// Whether sandboxed commands may use the network. A project's file may only turn it off.
    pub network: Option<bool>,
    /// The MCP servers to turn on.
    pub mcp: Option<Vec<String>>,
    /// Models a person may choose for a thread besides `model`, as `[[models]]`.
    pub models: Option<Vec<ModelEntry>>,
    /// Whether each new thread works in a worktree of its own.
    pub worktree: Option<bool>,
    /// Whether a model titles each thread after its first exchange.
    pub titles: Option<bool>,
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
    /// Where spans and metrics go. Only the person's or the server's own file sets it.
    pub telemetry: Option<TelemetryFile>,
    /// The data store. Only the person's or the server's own file sets it.
    pub store: Option<StoreFile>,
    /// A session server elsewhere this CLI is a client of. Only the person's own file sets
    /// it: their sign-in goes there.
    pub server: Option<ServerFile>,
    /// A cloud contract's models, in place of `GLAVE_BASE_URL`'s server. Only the person's or
    /// the server's own file sets it: the organization's credentials go there.
    pub provider: Option<ProviderFile>,
    /// The GitHub App people sign in to GitHub through, in the browser. Only the person's or
    /// the server's own file sets it: their GitHub sign-in goes there.
    pub github: Option<GithubFile>,
}

/// `[github]`: the GitHub App whose device flow signs people in to GitHub from the browser
/// (`session.md`, "Git hosts").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubFile {
    /// The app's client id: public, not a secret. The app must have device flow turned on.
    pub client_id: String,
    /// The app's name in its URL (`https://github.com/apps/<app>`), for the page where people
    /// choose which repositories it may use.
    pub app: Option<String>,
    /// Where GitHub is: `https://github.com` unless set, such as a GitHub Enterprise Server.
    pub url: Option<String>,
}

impl GithubFile {
    /// What is wrong with it, if anything.
    fn problem(&self) -> Option<String> {
        let id = self.client_id.trim();
        if id.is_empty() || id.chars().any(|c| !c.is_ascii_alphanumeric() && c != '.') {
            return Some(format!(
                "client_id = {:?} is not an app's client id",
                self.client_id
            ));
        }
        if let Some(app) = &self.app
            && (app.trim().is_empty()
                || app.chars().any(|c| !c.is_ascii_alphanumeric() && c != '-'))
        {
            return Some(format!(
                "app = {app:?} is not an app's name as its URL gives it"
            ));
        }
        let url = self.url.as_ref()?;
        CredentialHost::named(url, Forge::Github).is_none().then(|| {
            format!(
                "url = {url:?} is not a GitHub server's address: https:// and its host, such as \
                 https://github.example.com (http only to this computer)"
            )
        })
    }
}

/// `[server]`: a session server elsewhere (`glave --server`), such as an organization's.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerFile {
    /// Where it is, such as `https://glave.example.com`: https, or http only on this
    /// computer.
    pub url: String,
}

/// `[store]`: the one data store (AGENTS.md, "One of each"). The local SQLite file in glave's
/// folder unless `url` names a Postgres database.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreFile {
    /// `postgres://user:password@host/database`; TLS off this computer unless
    /// `sslmode=disable`.
    pub url: String,
}

impl StoreFile {
    /// Whether `url` names a Postgres database.
    pub fn postgres(url: &str) -> bool {
        let url = url.trim();
        url.starts_with("postgres://") || url.starts_with("postgresql://")
    }
}

/// `[telemetry]`: an OpenTelemetry collector to send spans and metrics to. Off unless set;
/// never content: no prompts, answers, file contents or secrets.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryFile {
    /// The collector's OTLP/HTTP endpoint, such as `http://localhost:4318`.
    pub otlp: String,
}

/// One of `[[models]]`: a model offered for a thread besides the default, at the same
/// endpoint, as `model/list` shows it (`session.md`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    /// The model's id, as `model` would name it.
    pub id: String,
    /// A name to show for it.
    pub name: Option<String>,
    /// Whether this model is Hope, glave's own, for clients to show as such.
    #[serde(default)]
    pub hope: bool,
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
        safe_to_send(&self.url, &[])
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
    #[serde(default)]
    pub placement: PlacementRules,
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
    pub modes: Option<Vec<ApprovalMode>>,
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
    /// Servers vouched for: remote ones and a project's, untrusted by default, are trusted.
    #[serde(default)]
    pub trusted: Vec<String>,
}

/// `[budget]`.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetRules {
    pub person_per_day_usd: Option<f64>,
}

/// `config.toml` at `path`, from its `text`.
pub fn config(text: &str, path: &Path) -> crate::Result<ConfigFile> {
    let file: ConfigFile = toml::from_str(text)
        .map_err(|error| crate::Error::Invalid(format!("{}: {error}", path.display())))?;
    if let Some(hooks) = &file.hooks {
        Hooks::new(hooks.hooks())
            .map_err(|error| crate::Error::Invalid(format!("{}: {error}", path.display())))?;
    }
    if let Some(gateway) = &file.gateway
        && !gateway.safe()
    {
        invalid!(
            "{}: [gateway] url = {:?} is not https (http only to this computer): your sign-in \
             goes there",
            path.display(),
            gateway.url
        );
    }
    if let Some(telemetry) = &file.telemetry {
        let url = telemetry.otlp.trim();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            invalid!(
                "{}: [telemetry] otlp = {:?} is not an http or https URL",
                path.display(),
                telemetry.otlp
            );
        }
    }
    if let Some(problem) = file.github.as_ref().and_then(GithubFile::problem) {
        invalid!("{}: [github] {problem}", path.display());
    }
    if let Some(server) = &file.server
        && !safe_to_send(&server.url, &[])
    {
        invalid!(
            "{}: [server] url = {:?} is not https (http only to this computer): your sign-in \
             goes there",
            path.display(),
            server.url
        );
    }
    if let Some(store) = &file.store
        && !StoreFile::postgres(&store.url)
    {
        invalid!(
            "{}: [store] url = {:?} is not a postgres:// URL",
            path.display(),
            crate::policy::redacted(&store.url)
        );
    }
    if let Some(web) = &file.web {
        web.policy().map_err(|error| {
            crate::Error::Invalid(format!("{}: {}", path.display(), error.chain()))
        })?;
    }
    Ok(file)
}

/// `policy.toml` at `path`, from its `text`: a version glave knows, and globs that compile.
pub fn policy(text: &str, path: &Path) -> crate::Result<PolicyFile> {
    let file: PolicyFile = toml::from_str(text)
        .map_err(|error| crate::Error::Invalid(format!("{}: {error}", path.display())))?;
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
    if let Some(usd) = file.budget.person_per_day_usd
        && !(usd.is_finite() && usd >= 0.0)
    {
        invalid!(
            "{}: [budget] person_per_day_usd = {usd} is not an amount of dollars",
            path.display()
        );
    }
    Ok(file)
}
