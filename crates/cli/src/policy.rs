//! Config and policy (`specs/policy.md`): choices from flags, the environment and `config.toml`
//! files, nearest wins; constraints from policy files, which only narrow; and the two together,
//! where a choice outside the constraints is an error naming the rule and where it came from.

mod choices;
mod effective;
mod explain;
mod files;
mod rules;
#[cfg(test)]
pub(crate) mod tests;

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context as _;

pub use choices::{Choices, Chosen, Layer, Level, Source};
pub use effective::{check_model, check_own_servers};
pub use explain::explain;
pub use files::{ConfigFile, SandboxChoice};
pub use rules::Constraints;

/// The choices file, in glave's folder and in a project's `.glave/`.
pub const CONFIG: &str = "config.toml";

/// The organization's policy, as device management installs it.
pub const ORGANIZATION_POLICY: &str = "/etc/glave/policy.toml";

/// Where the files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
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
    /// The project's `config.toml`, present but not used: a project's settings count only once
    /// the person trusts the project, which glave cannot ask yet.
    pub untrusted: Option<PathBuf>,
}

impl Loaded {
    /// The choices within the constraints, or why they cannot be.
    pub fn settle(&self) -> anyhow::Result<Choices> {
        effective::settle(self.choices.clone(), &self.constraints)
    }
}

/// Reads the files at `locations` under the choices of `flags` and `env`.
pub async fn load(
    locations: &Locations,
    flags: ConfigFile,
    env: ConfigFile,
) -> anyhow::Result<Loaded> {
    let person = match read(&locations.person).await? {
        Some(text) => Some(files::config(&text, &locations.person)?),
        None => None,
    };
    let untrusted = tokio::fs::try_exists(&locations.project)
        .await
        .unwrap_or(false)
        .then(|| locations.project.clone());
    let mut policies = Vec::new();
    for path in &locations.policies {
        if let Some(text) = read(path).await? {
            policies.push((path.clone(), files::policy(&text, path)?));
        }
    }
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
    if let Some(values) = person {
        layers.push(Layer {
            level: Level::File(locations.person.clone()),
            values,
        });
    }
    Ok(assemble(&layers, &policies, untrusted))
}

/// The choices of `layers`, nearest first, under the rules of `policies`.
pub fn assemble(
    layers: &[Layer],
    policies: &[(PathBuf, files::PolicyFile)],
    untrusted: Option<PathBuf>,
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
        untrusted,
    }
}

/// The text of `path`, or nothing when there is no such file.
async fn read(path: &Path) -> anyhow::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}
