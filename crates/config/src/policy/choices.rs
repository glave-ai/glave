//! Choices, nearest wins: flags, then the environment, then the project's file, then the
//! person's, then glave's defaults. Each value remembers where it came from.

use std::fmt;
use std::path::PathBuf;

use glave_harness::{ApprovalMode, SandboxLevel};

use crate::policy::files::{Approval, ConfigFile, ModelEntry, SandboxChoice};
use crate::policy::memory::{MemoryChoice, SpaceChoice};
use crate::policy::provider::ProviderFile;
use crate::policy::web::WebFile;
use crate::policy::workers::{On, WorkersFile};
use glave_memory::SpaceMemory;

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Flag(&'static str),
    Env(&'static str),
    File(PathBuf),
    Default,
    /// glave's default was not allowed, so this is the value the policy file leaves.
    Narrowed(PathBuf),
    /// glave's default sandbox cannot run on this computer, for this reason.
    Unavailable(String),
    /// glave's default approval mode when commands run without a sandbox.
    Unsandboxed,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flag(flag) => f.write_str(flag),
            Self::Env(name) => f.write_str(name),
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Default => f.write_str("default"),
            Self::Narrowed(path) => write!(f, "default, narrowed by {}", path.display()),
            Self::Unavailable(why) => write!(f, "default, but it cannot run here: {why}"),
            Self::Unsandboxed => f.write_str("default when commands run without a sandbox"),
        }
    }
}

/// A value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen<T> {
    pub value: T,
    pub from: Source,
}

/// The model that judges flagged actions in auto mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judge {
    /// The main model.
    Main,
    Off,
    Model(String),
}

impl Judge {
    /// `judge_model` as written: `off`, a model, or nothing (the main model).
    pub fn named(text: &str) -> Self {
        match text.trim() {
            "" => Self::Main,
            "off" => Self::Off,
            model => Self::Model(model.to_owned()),
        }
    }

    /// The judge's model when the main one is `main`, or none when it is off.
    pub fn model(&self, main: &str) -> Option<String> {
        match self {
            Self::Main => Some(main.to_owned()),
            Self::Off => None,
            Self::Model(model) => Some(model.clone()),
        }
    }
}

impl fmt::Display for Judge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Main => f.write_str("the main model"),
            Self::Off => f.write_str("off"),
            Self::Model(model) => f.write_str(model),
        }
    }
}

/// Every choice, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// No default: glave says how to choose one when there is none.
    pub model: Option<Chosen<String>>,
    pub judge: Chosen<Judge>,
    pub approval: Chosen<ApprovalMode>,
    /// `None` is no sandbox. `project` by default, wherever it can run.
    pub sandbox: Chosen<Option<SandboxLevel>>,
    /// Whether sandboxed commands may use the network: on by default.
    pub network: Chosen<bool>,
    /// The MCP servers to turn on; every server in `mcp.json` when not chosen.
    pub mcp: Option<Chosen<Vec<String>>>,
    /// Models offered for a thread besides `model`; none when not chosen.
    pub models: Option<Chosen<Vec<ModelEntry>>>,
    /// Whether each new thread works in a worktree of its own; off, in the folder itself.
    pub worktree: Chosen<bool>,
    /// Where new threads work: here by default.
    pub on: Chosen<On>,
    /// The backend workers run on, if one is set up.
    pub workers: Option<Chosen<WorkersFile>>,
    /// The gateway elsewhere model calls go to, if any: its URL.
    pub gateway: Option<Chosen<String>>,
    /// What `web_fetch` may reach: on, public hosts only, by default.
    pub web: Chosen<WebFile>,
    /// Whether glave reads and writes memory for this person: on by default.
    pub memory: Chosen<bool>,
    /// This space's memory setting: shared by default.
    pub space_memory: Chosen<SpaceMemory>,
    /// The OpenTelemetry collector spans and metrics go to, if any: its OTLP/HTTP URL.
    pub telemetry: Option<Chosen<String>>,
    /// The Postgres database that is the data store, if any; the local SQLite file if not.
    /// The URL may hold a password: show it only [`redacted`](crate::policy::redacted).
    pub store: Option<Chosen<String>>,
    /// The session server elsewhere this CLI is a client of, if any: its URL.
    pub server: Option<Chosen<String>>,
    /// A cloud contract's models, if any, in place of `GLAVE_BASE_URL`'s server.
    pub provider: Option<Chosen<ProviderFile>>,
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
                "approval" => "--approval",
                "sandbox" => "--sandbox",
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
pub fn resolve(layers: &[Layer]) -> Choices {
    Choices {
        model: nearest(layers, "model", |file| file.model.clone()),
        judge: defaulted(
            nearest(layers, "judge_model", |file| {
                file.judge_model.as_deref().map(Judge::named)
            }),
            Judge::Main,
        ),
        approval: defaulted(
            nearest(layers, "approval", |file| file.approval.map(Approval::mode)),
            ApprovalMode::default(),
        ),
        sandbox: defaulted(
            nearest(layers, "sandbox", |file| {
                file.sandbox.map(SandboxChoice::level)
            }),
            Some(SandboxLevel::Project),
        ),
        network: defaulted(nearest(layers, "network", |file| file.network), true),
        mcp: nearest(layers, "mcp", |file| file.mcp.clone()),
        models: nearest(layers, "models", |file| file.models.clone()),
        worktree: defaulted(nearest(layers, "worktree", |file| file.worktree), false),
        on: defaulted(nearest(layers, "on", |file| file.on), On::Here),
        workers: nearest(layers, "workers", |file| file.workers.clone()),
        server: nearest(layers, "server", |file| {
            file.server
                .as_ref()
                .map(|server| server.url.trim().to_owned())
        }),
        provider: nearest(layers, "provider", |file| file.provider.clone()),
        store: nearest(layers, "store", |file| {
            file.store.as_ref().map(|store| store.url.trim().to_owned())
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
        memory: defaulted(
            nearest(layers, "memory", |file| {
                file.memory.map(|choice| choice == MemoryChoice::On)
            }),
            true,
        ),
        space_memory: defaulted(
            nearest(layers, "space_memory", |file| {
                file.space_memory.map(SpaceChoice::setting)
            }),
            SpaceMemory::Shared,
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
