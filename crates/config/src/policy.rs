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

use glave_event::Permissions;
use glave_ports::settings::Hook;

pub(crate) use choices::project_permissions;
pub use choices::{Choices, Chosen, Layer, Level, Source};
use effective::give_way;
pub use effective::{check_model, check_own_servers};
pub use explain::explain;
pub use files::{
    ConfigFile, GithubFile, HooksFile, ModelEntry, PolicyFile, ServeFile, ServerFile, StoreFile,
    TaskRules, TelemetryFile, config,
};
pub use memory::{MemorySettings, settle as memory_settings};
pub use placement::{Placement, PlacementRules};
pub use provider::{ApiChoice, BedrockEndpointChoice, ProviderFile};
pub use redact::{has_password, redacted};
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
    /// The levels the choices came from other than a project's, nearest first: what
    /// [`Loaded::with_project`] puts a project's among.
    pub layers: Vec<Layer>,
}

/// A project's trust, for `glave policy explain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrust {
    pub root: PathBuf,
    /// Each thing that needs trust, and where it stands.
    pub rows: Vec<[String; 2]>,
}

impl Loaded {
    /// The choices within the constraints, or why they cannot be: among them, where no
    /// sandbox can run here, a rule that requires one.
    pub fn settle(&self) -> crate::Result<Choices> {
        let mut choices = effective::settle(self.choices.clone(), &self.constraints)?;
        if let Some(why) = &self.no_sandbox_here {
            if let Some(rule) = self.constraints.sandbox_required() {
                crate::error::refuse!(
                    "the sandbox cannot run on this computer ({why}), and {rule} requires it"
                );
            }
            choices.unsandboxed = Some(why.clone());
        }
        Ok(choices)
    }

    /// The choices within the constraints for a server that already runs (`give_way`),
    /// and what gave way, one sentence each. Where no sandbox can run here, a rule that
    /// requires one leaves every command asking: Full access is not left to choose.
    pub fn settle_giving_way(&self) -> crate::Result<(Choices, Vec<String>)> {
        let (mut choices, aside) = give_way(self.choices.clone(), &self.constraints)?;
        choices.unsandboxed.clone_from(&self.no_sandbox_here);
        Ok((choices, aside))
    }

    /// The choices for a thread in a project whose `config.toml`, at `path`, applies as
    /// `project`: its choices come after the flags' and the environment's and before the
    /// person's, as [`load`] puts them, under the same constraints. A project's these already
    /// hold gives way to it.
    #[must_use]
    pub fn with_project(&self, path: &Path, project: ConfigFile) -> Self {
        let (highest, _) = self.constraints.highest_permissions();
        let layers = with_project(&self.layers, project_layer(path, project), highest);
        Self {
            choices: choices::resolve(&layers),
            configs: files_of(&layers),
            ..self.clone()
        }
    }
}

/// `own`, the levels other than a project's, with `project` among them: after the flags and
/// the environment, before the person's file. Its mode only narrows what `own` choose,
/// unless it gives the full access its trust lets it, under the cap `highest`
/// ([`project_permissions`]).
fn with_project(own: &[Layer], mut project: Layer, highest: Permissions) -> Vec<Layer> {
    let mine = choices::resolve(own).permissions.value;
    project.values.permissions = project_permissions(project.values.permissions, mine, highest);
    let at = own
        .iter()
        .position(|layer| matches!(layer.level, Level::File(_)))
        .unwrap_or(own.len());
    let mut layers = own.to_vec();
    layers.insert(at, project);
    layers
}

/// The layer of a project's `config.toml` at `path`, choosing `values` less what a project
/// never chooses. A project never chooses the gateway: the person's sign-in would go where it
/// said. Nor where telemetry goes,
/// nor where history is kept, nor which server to sign in to, nor where people reach this one,
/// nor which cloud the organization's credentials go to, nor which app their GitHub sign-in
/// goes to. It may take the network away from sandboxed commands, never give it back.
fn project_layer(path: &Path, values: ConfigFile) -> Layer {
    let values = ConfigFile {
        gateway: None,
        telemetry: None,
        store: None,
        server: None,
        serve: None,
        provider: None,
        github: None,
        network: values.network.filter(|on| !on),
        ..values
    };
    Layer {
        level: Level::File(path.to_owned()),
        values,
    }
}

/// The `config.toml` files among `layers`, nearest first.
fn files_of(layers: &[Layer]) -> Vec<PathBuf> {
    layers
        .iter()
        .filter_map(|layer| match &layer.level {
            Level::File(path) => Some(path.clone()),
            Level::Flags | Level::Env => None,
        })
        .collect()
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
        Some(text) => Some(config(&text, &locations.person)?),
        None => None,
    };
    let person_hooks = person
        .as_ref()
        .and_then(|file| file.hooks.as_ref())
        .map(HooksFile::hooks)
        .unwrap_or_default();
    let mut own = vec![
        Layer {
            level: Level::Flags,
            values: flags,
        },
        Layer {
            level: Level::Env,
            values: env,
        },
    ];
    if let Some(values) = person {
        own.push(Layer {
            level: Level::File(locations.person.clone()),
            values,
        });
    }
    let mut loaded = match project {
        Some(values) => {
            let (highest, _) = Constraints::of(policies).highest_permissions();
            let project = project_layer(&locations.project, values);
            let layers = with_project(&own, project, highest);
            assemble(&layers, policies, None)
        }
        None => assemble(&own, policies, None),
    };
    loaded.layers = own;
    loaded.person_hooks = person_hooks;
    Ok(loaded)
}

/// The choices of `layers`, nearest first, under the rules of `policies`.
pub fn assemble(
    layers: &[Layer],
    policies: &[(PathBuf, PolicyFile)],
    project: Option<ProjectTrust>,
) -> Loaded {
    Loaded {
        choices: choices::resolve(layers),
        constraints: Constraints::of(policies),
        configs: files_of(layers),
        policies: policies.iter().map(|(path, _)| path.clone()).collect(),
        project,
        person_hooks: Vec::new(),
        no_sandbox_here: None,
        layers: layers.to_vec(),
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
