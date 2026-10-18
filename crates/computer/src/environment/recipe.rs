//! `.glave/environment.toml`: glave's own way to write an environment down.

use std::collections::BTreeMap;

use glave_ports::computer::{Environment, Image, Network, Size};
use serde::Deserialize;

use super::{EnvironmentError, is_variable_name};

/// The file as written. Unknown keys are refused, so a typo is never quietly ignored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    image: Option<String>,
    setup: Option<Setup>,
    cpus: Option<u32>,
    memory_gb: Option<u32>,
    disk_gb: Option<u32>,
    network: Option<Network>,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    secrets: Vec<String>,
    idle_minutes: Option<u32>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

/// One setup command, or several in order.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Setup {
    One(String),
    Many(Vec<String>),
}

/// Reads an environment.toml. The image defaults to the host.
pub fn parse_environment_toml(text: &str) -> Result<Environment, EnvironmentError> {
    let file: File =
        toml::from_str(text).map_err(|error| EnvironmentError::invalid(error.to_string()))?;
    let image = match file.image.as_deref() {
        None => Image::Host,
        Some(name) => {
            Image::named(name).ok_or_else(|| EnvironmentError::invalid("the image is empty"))?
        }
    };
    let setup = match file.setup {
        None => Vec::new(),
        Some(Setup::One(command)) => vec![command],
        Some(Setup::Many(commands)) => commands,
    };
    if setup.iter().any(|command| command.trim().is_empty()) {
        return Err(EnvironmentError::invalid("a setup command is empty"));
    }
    let environment = Environment {
        image,
        setup,
        size: Size {
            cpus: file.cpus,
            memory_gb: file.memory_gb,
            disk_gb: file.disk_gb,
        },
        network: file.network,
        allow: file.allow,
        secrets: file.secrets,
        idle_minutes: file.idle_minutes,
        env: file.env,
    };
    check(&environment)?;
    Ok(environment)
}

/// What every environment must be, however it was written.
pub(crate) fn check(environment: &Environment) -> Result<(), EnvironmentError> {
    if !environment.allow.is_empty() && environment.network != Some(Network::Allowlist) {
        return Err(EnvironmentError::invalid(
            "allow lists hosts only when network = \"allowlist\"",
        ));
    }
    if environment.allow.iter().any(|host| host.trim().is_empty()) {
        return Err(EnvironmentError::invalid("an allowed host is empty"));
    }
    if let Some(name) = environment
        .secrets
        .iter()
        .chain(environment.env.keys())
        .find(|name| !is_variable_name(name))
    {
        return Err(EnvironmentError::invalid(format!(
            "{name:?} is not a variable name"
        )));
    }
    if let Some(name) = environment
        .secrets
        .iter()
        .find(|name| environment.env.contains_key(*name))
    {
        return Err(EnvironmentError::invalid(format!(
            "{name} is a secret, so its value comes through the credential helper, never env"
        )));
    }
    Ok(())
}
