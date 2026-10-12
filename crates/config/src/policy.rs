//! Config and policy (`specs/policy.md`): choices from flags, the environment and `config.toml`
//! files, nearest wins; constraints from policy files, which only narrow; and the two together,
//! where a choice outside the constraints is an error naming the rule and where it came from.

mod choices;
mod effective;
mod explain;
mod files;
mod memory;
mod placement;
mod provider;
mod redact;
mod rules;
#[cfg(test)]
pub(crate) mod tests;
mod web;
mod workers;

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use glave_harness::Hook;

pub use choices::{Choices, Chosen, Judge, Layer, Level, Source};
pub use effective::{check_model, check_own_servers, thread_modes, unsandboxed};
pub use explain::explain;
pub use files::{
    ConfigFile, HooksFile, ModelEntry, PolicyFile, SandboxChoice, ServerFile, StoreFile, TaskRules,
    TelemetryFile, config,
};
pub use memory::{MemorySettings, settle as memory_settings};
pub use placement::{Placement, PlacementRules};
pub use provider::{ApiChoice, BedrockEndpointChoice, ProviderFile};
pub use redact::redacted;
pub use rules::Constraints;
pub use web::WebFile;
pub use workers::{Backend, On, WorkersFile};

/// The choices file, in glave's folder and in a project's `.glave/`.
pub const CONFIG: &str = "config.toml";

/// The organization's policy, as device management installs it.
pub const ORGANIZATION_POLICY: &str = "/etc/glave/policy.toml";

/// Where the files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// The project's root: the repository around the folder, or the folder itself.
    pub root: PathBuf,
    /// The person's `config.toml`.
    pub person: PathBuf,
    /// The project's `.glave/config.toml`.
    pub project: PathBuf,
    /// Policy files, organization first.
    pub policies: Vec<PathBuf>,
}

impl Locations {
    /// The files for glave's folder `home`, working in `folder`: the project is the repository
    /// around `folder`, or `folder` itself outside one.
    pub fn here(home: &Path, folder: &Path) -> Self {
        let project = folder
            .ancestors()
            .find(|dir| dir.join(".git").exists())
            .unwrap_or(folder);
        Self {
            root: project.to_owned(),
            person: home.join(CONFIG),
            project: project.join(".glave").join(CONFIG),
            policies: vec![PathBuf::from(ORGANIZATION_POLICY)],
        }
    }
}

/// Everything read, before the choices are kept within the constraints.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub choices: Choices,
    pub constraints: Constraints,
    /// The `config.toml` files whose choices count, nearest first.
    pub configs: Vec<PathBuf>,
    /// The policy files read.
    pub policies: Vec<PathBuf>,
    /// What the project brings that needs trust, and where each thing stands.
    pub project: Option<ProjectTrust>,
    /// The hooks in the person's `config.toml`, which are theirs and need no trust.
    pub person_hooks: Vec<Hook>,
    /// Why this computer cannot run the sandbox, when that is known before trying.
    pub no_sandbox_here: Option<String>,
}

/// A project's trust, for `glave policy explain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrust {
    pub root: PathBuf,
    /// Each thing that needs trust, and where it stands.
    pub rows: Vec<[String; 2]>,
}

impl Loaded {
    /// The choices within the constraints, or why they cannot be.
    pub fn settle(&self) -> crate::Result<Choices> {
        let choices = effective::settle(self.choices.clone(), &self.constraints)?;
        Ok(match &self.no_sandbox_here {
            Some(why) => effective::unsandboxed(choices, why, &self.constraints),
            None => choices,
        })
    }
}

/// The policy files that exist at `locations`, organization first.
pub async fn policies(locations: &Locations) -> crate::Result<Vec<(PathBuf, PolicyFile)>> {
    policy_files(&locations.policies).await
}

/// The rules of one policy file, `text`, as if it were at `path`.
pub fn constraints(text: &str, path: &Path) -> crate::Result<Constraints> {
    Ok(Constraints::of(&[(
        path.to_owned(),
        files::policy(text, path)?,
    )]))
}

/// The files of `paths` that exist, read as policies, in order.
pub async fn policy_files(paths: &[PathBuf]) -> crate::Result<Vec<(PathBuf, PolicyFile)>> {
    let mut policies = Vec::new();
    for path in paths {
        if let Some(text) = read(path).await? {
            policies.push((path.clone(), files::policy(&text, path)?));
        }
    }
    Ok(policies)
}

/// Reads the config files at `locations` under the choices of `flags` and `env`, with the
/// `project` choices that count (the project's `config.toml` as far as it is trusted).
pub async fn load(
    locations: &Locations,
    policies: &[(PathBuf, PolicyFile)],
    project: Option<ConfigFile>,
    flags: ConfigFile,
    env: ConfigFile,
) -> crate::Result<Loaded> {
    let person = match read(&locations.person).await? {
        Some(text) => Some(files::config(&text, &locations.person)?),
        None => None,
    };
    let person_hooks = person
        .as_ref()
        .and_then(|file| file.hooks.as_ref())
        .map(HooksFile::hooks)
        .unwrap_or_default();
    let mut layers = vec![
        Layer {
            level: Level::Flags,
            values: flags,
        },
        Layer {
            level: Level::Env,
            values: env,
        },
    ];
    if let Some(values) = project {
        // A project never chooses the gateway: the person's sign-in would go where it said.
        // Nor whether the person's memory is on: that is theirs alone. Nor where telemetry
        // goes, nor where history is kept, nor which server to sign in to, nor which cloud the
        // organization's credentials go to. It may take the network away from sandboxed
        // commands, never give it back.
        let values = ConfigFile {
            gateway: None,
            memory: None,
            telemetry: None,
            store: None,
            server: None,
            provider: None,
            network: values.network.filter(|on| !on),
            ..values
        };
        layers.push(Layer {
            level: Level::File(locations.project.clone()),
            values,
        });
    }
    if let Some(values) = person {
        layers.push(Layer {
            level: Level::File(locations.person.clone()),
            values,
        });
    }
    let mut loaded = assemble(&layers, policies, None);
    loaded.person_hooks = person_hooks;
    Ok(loaded)
}

/// The choices of `layers`, nearest first, under the rules of `policies`.
pub fn assemble(
    layers: &[Layer],
    policies: &[(PathBuf, files::PolicyFile)],
    project: Option<ProjectTrust>,
) -> Loaded {
    Loaded {
        choices: choices::resolve(layers),
        constraints: Constraints::of(policies),
        configs: layers
            .iter()
            .filter_map(|layer| match &layer.level {
                Level::File(path) => Some(path.clone()),
                Level::Flags | Level::Env => None,
            })
            .collect(),
        policies: policies.iter().map(|(path, _)| path.clone()).collect(),
        project,
        person_hooks: Vec::new(),
        no_sandbox_here: None,
    }
}

/// The text of `path`, or nothing when there is no such file.
async fn read(path: &Path) -> crate::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::Error::io("read", path)(error)),
    }
}
