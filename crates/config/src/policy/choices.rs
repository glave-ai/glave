//! Choices, nearest wins: flags, then the environment, then the project's file, then the
//! person's, then glave's defaults. Each value remembers where it came from.

use std::fmt;
use std::path::PathBuf;

use glave_event::Permissions;
use glave_ports::host::PublicOrigin;
use glave_ports::memories::ProjectMemory;
use glave_ports::settings::Judge;

use crate::policy::files::{ConfigFile, CreditsFile, EmailFile, GithubFile, ModelEntry};
use crate::policy::memory::ProjectChoice;
use crate::policy::provider::ProviderFile;
use crate::policy::web::WebFile;
use crate::policy::workers::{ComputersFile, On, WorkersFile};

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Flag(&'static str),
    Env(&'static str),
    File(PathBuf),
    Default,
    /// glave's default was not allowed, so this is the value the policy file leaves.
    Narrowed(PathBuf),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flag(flag) => f.write_str(flag),
            Self::Env(name) => f.write_str(name),
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Default => f.write_str("default"),
            Self::Narrowed(path) => write!(f, "default, narrowed by {}", path.display()),
        }
    }
}

/// A value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen<T> {
    pub value: T,
    pub from: Source,
}

/// Every choice, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// No default: glave says how to choose one when there is none.
    pub model: Option<Chosen<String>>,
    pub judge: Chosen<Judge>,
    /// What the agent may do without asking: Workspace by default.
    pub permissions: Chosen<Permissions>,
    /// Why the sandbox that lets commands change the project cannot run on this computer, once
    /// glave knows: Workspace then runs without one, and every change and command asks.
    pub unsandboxed: Option<String>,
    /// Whether sandboxed commands may use the network: on by default.
    pub network: Chosen<bool>,
    /// The MCP servers to turn on; every server in `mcp.json` when not chosen.
    pub mcp: Option<Chosen<Vec<String>>>,
    /// Models offered for a thread besides `model`; none when not chosen.
    pub models: Option<Chosen<Vec<ModelEntry>>>,
    /// Whether each new thread works in a worktree of its own; off, in the folder itself.
    pub worktree: Chosen<bool>,
    /// Whether a model titles each thread after its first exchange: on by default; off, a
    /// thread's title is the first line of its first message.
    pub titles: Chosen<bool>,
    /// Where new threads work: here by default.
    pub on: Chosen<On>,
    /// The backend workers run on, if one is set up.
    pub workers: Option<Chosen<WorkersFile>>,
    /// Each person's own computer, if people have one; none, threads work on the server's.
    pub computers: Option<Chosen<ComputersFile>>,
    /// The gateway elsewhere model calls go to, if any: its URL.
    pub gateway: Option<Chosen<String>>,
    /// What `web_fetch` may reach: on, public hosts only, by default.
    pub web: Chosen<WebFile>,
    /// This project's memory setting: shared by default.
    pub project_memory: Chosen<ProjectMemory>,
    /// The OpenTelemetry collector spans and metrics go to, if any: its OTLP/HTTP URL.
    pub telemetry: Option<Chosen<String>>,
    /// The Postgres database that is the data store, if any; the local SQLite file if not.
    /// The URL may hold a password: show it only [`redacted`](crate::policy::redacted).
    pub store: Option<Chosen<String>>,
    /// The S3-compatible bucket blobs' bytes are kept in, if any; a folder in glave's folder
    /// if not.
    pub bucket: Option<Chosen<String>>,
    /// The session server elsewhere this CLI is a client of, if any: its URL.
    pub server: Option<Chosen<String>>,
    /// Where people reach this glave from elsewhere, if they do: its public origin, for which a
    /// tunnel or a proxy serves HTTPS, or glave itself. Without one, glave answers only on this
    /// computer.
    pub origin: Option<Chosen<PublicOrigin>>,
    /// The ACME directory glave's own certificate comes from, when it serves HTTPS itself.
    pub acme: Option<Chosen<String>>,
    /// A cloud contract's models, if any, in place of `GLAVE_BASE_URL`'s server.
    pub provider: Option<Chosen<ProviderFile>>,
    /// The GitHub app people sign in to GitHub through in the browser, if one is set up; glave's
    /// own otherwise.
    pub github: Option<Chosen<GithubFile>>,
    /// The mail server glave sends through, if one is set up.
    pub email: Option<Chosen<EmailFile>>,
    /// Whether anyone may sign up at the public origin with their email (`[serve] signup`).
    pub signup: Option<Chosen<bool>>,
    /// Whether organizations pay for their models from credits (`[credits]`), and with what
    /// monthly limit.
    pub credits: Option<Chosen<CreditsFile>>,
}

/// One level choices come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Level {
    Flags,
    Env,
    /// A `config.toml`: the project's or the person's.
    File(PathBuf),
}

/// The choices one level makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub level: Level,
    pub values: ConfigFile,
}

impl Layer {
    /// Where this level's `key` came from: the flag or variable that sets it, or the file.
    fn source(&self, key: &'static str) -> Source {
        match &self.level {
            Level::Flags => Source::Flag(match key {
                "model" => "--model",
                "permissions" => "--permissions",
                "network" => "--network or --no-network",
                "worktree" => "--worktree",
                "on" => "--on",
                _ => "a flag",
            }),
            Level::Env => Source::Env(match key {
                "model" => "GLAVE_MODEL",
                "judge_model" => "GLAVE_JUDGE_MODEL",
                _ => "the environment",
            }),
            Level::File(path) => Source::File(path.clone()),
        }
    }
}

/// The choices of `layers`, nearest first: for each key, the first level that sets it wins.
pub(super) fn resolve(layers: &[Layer]) -> Choices {
    Choices {
        model: nearest(layers, "model", |file| file.model.clone()),
        judge: defaulted(
            nearest(layers, "judge_model", |file| {
                file.judge_model.as_deref().map(Judge::named)
            }),
            Judge::Main,
        ),
        permissions: defaulted(
            nearest(layers, "permissions", |file| file.permissions),
            Permissions::default(),
        ),
        unsandboxed: None,
        network: defaulted(nearest(layers, "network", |file| file.network), true),
        mcp: nearest(layers, "mcp", |file| file.mcp.clone()),
        models: nearest(layers, "models", |file| file.models.clone()),
        worktree: defaulted(nearest(layers, "worktree", |file| file.worktree), false),
        titles: defaulted(nearest(layers, "titles", |file| file.titles), true),
        on: defaulted(nearest(layers, "on", |file| file.on), On::Here),
        workers: nearest(layers, "workers", |file| file.workers.clone()),
        computers: nearest(layers, "computers", |file| file.computers.clone()),
        server: nearest(layers, "server", |file| {
            file.server
                .as_ref()
                .map(|server| server.url.trim().to_owned())
        }),
        origin: nearest(layers, "serve", |file| file.serve.as_ref()?.public_origin()),
        acme: nearest(layers, "serve", |file| file.serve.as_ref()?.directory()),
        provider: nearest(layers, "provider", |file| file.provider.clone()),
        github: nearest(layers, "github", |file| file.github.clone()),
        email: nearest(layers, "email", |file| file.email.clone()),
        signup: nearest(layers, "serve", |file| file.serve.as_ref()?.signup),
        credits: nearest(layers, "credits", |file| file.credits.clone()),
        store: nearest(layers, "store", |file| {
            let url = file.store.as_ref()?.url.as_ref()?;
            Some(url.trim().to_owned())
        }),
        bucket: nearest(layers, "bucket", |file| {
            let bucket = file.store.as_ref()?.bucket.as_ref()?;
            Some(bucket.trim().to_owned())
        }),
        telemetry: nearest(layers, "telemetry", |file| {
            file.telemetry
                .as_ref()
                .map(|telemetry| telemetry.otlp.trim().to_owned())
        }),
        gateway: nearest(layers, "gateway", |file| {
            file.gateway
                .as_ref()
                .map(|gateway| gateway.url.trim().to_owned())
        }),
        web: defaulted(
            nearest(layers, "web", |file| file.web.clone()),
            WebFile::default(),
        ),
        project_memory: defaulted(
            nearest(layers, "project_memory", |file| {
                file.project_memory.map(ProjectChoice::setting)
            }),
            ProjectMemory::Shared,
        ),
    }
}

/// `chosen`, or glave's default `value`.
fn defaulted<T>(chosen: Option<Chosen<T>>, value: T) -> Chosen<T> {
    chosen.unwrap_or(Chosen {
        value,
        from: Source::Default,
    })
}

/// The value of `key` from the nearest level that sets it.
fn nearest<T>(
    layers: &[Layer],
    key: &'static str,
    get: impl Fn(&ConfigFile) -> Option<T>,
) -> Option<Chosen<T>> {
    layers.iter().find_map(|layer| {
        get(&layer.values).map(|value| Chosen {
            value,
            from: layer.source(key),
        })
    })
}

/// The mode a project's `config.toml` gives its threads where the levels other than the
/// project's choose `own`, under the policy's cap `highest`: a narrower one, or the same, at
/// once; full access only when it is there at all, since it stays out of a project's file until
/// the person trusts the project (`trust::Project::config_without_trust`), and then no higher
/// than the cap. A wider one is ignored, so a repository never widens what the person chose.
pub(crate) fn project_permissions(
    project: Option<Permissions>,
    own: Permissions,
    highest: Permissions,
) -> Option<Permissions> {
    project
        .filter(|mode| *mode == Permissions::FullAccess || *mode <= own)
        .map(|mode| mode.min(highest))
}
