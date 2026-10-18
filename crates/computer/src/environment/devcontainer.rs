//! The part of devcontainer.json glave can honour: the image, its environment variables, the
//! commands that set it up, and the size it needs. What would change the machine in ways
//! glave cannot carry out (building images, features, mounts, host commands) is refused by
//! name; settings for editors and port previews are left alone and listed.

use std::collections::BTreeMap;

use glave_ports::computer::{Environment, Image, Size};
use serde_json::{Map, Value};

use super::EnvironmentError;
use super::jsonc::strip;
use super::recipe::check;

/// Settings for an editor or a preview, which glave leaves alone.
const IGNORED: &[&str] = &[
    "$schema",
    "name",
    "customizations",
    "forwardPorts",
    "portsAttributes",
    "otherPortsAttributes",
    "remoteUser",
    "containerUser",
    "updateRemoteUserUID",
    "userEnvProbe",
    "overrideCommand",
    "shutdownAction",
    "waitFor",
    "workspaceFolder",
];

/// Settings glave cannot honour, and why.
const REFUSED: &[(&str, &str)] = &[
    (
        "build",
        "glave does not build images; publish the image and name it in \"image\"",
    ),
    (
        "dockerFile",
        "glave does not build images; publish the image and name it in \"image\"",
    ),
    ("context", "glave does not build images"),
    (
        "dockerComposeFile",
        "Docker Compose setups are not supported",
    ),
    ("service", "Docker Compose setups are not supported"),
    ("runServices", "Docker Compose setups are not supported"),
    (
        "initializeCommand",
        "it runs on the host machine, outside the computer",
    ),
    (
        "postStartCommand",
        "a computer keeps files, not running programs, so it would not rerun",
    ),
    (
        "postAttachCommand",
        "a computer keeps files, not running programs, so it would not rerun",
    ),
    ("mounts", "a computer's files come only from its sources"),
    (
        "workspaceMount",
        "a computer's files come only from its sources",
    ),
    (
        "runArgs",
        "container options are set by policy, not the project",
    ),
    (
        "privileged",
        "container options are set by policy, not the project",
    ),
    (
        "capAdd",
        "container options are set by policy, not the project",
    ),
    (
        "securityOpt",
        "container options are set by policy, not the project",
    ),
    (
        "init",
        "container options are set by policy, not the project",
    ),
    ("appPort", "port previews come later"),
];

/// Commands that set a computer up, in the order a devcontainer runs them.
const SETUP: [&str; 3] = [
    "onCreateCommand",
    "updateContentCommand",
    "postCreateCommand",
];

/// Reads a devcontainer.json: the environment, and the settings left alone.
pub fn parse_devcontainer(text: &str) -> Result<(Environment, Vec<String>), EnvironmentError> {
    let value: Value = serde_json::from_str(&strip(text))
        .map_err(|error| EnvironmentError::invalid(error.to_string()))?;
    let Value::Object(fields) = value else {
        return Err(EnvironmentError::invalid("the file is not a JSON object"));
    };
    let mut ignored = Vec::new();
    for key in fields.keys() {
        if let Some((_, why)) = REFUSED.iter().find(|(name, _)| name == key) {
            return Err(EnvironmentError::unsupported(key, why));
        }
        if IGNORED.contains(&key.as_str()) {
            ignored.push(key.clone());
        } else if !is_honoured(key) {
            return Err(EnvironmentError::unsupported(
                key,
                "glave does not know this setting, and does not skip what it does not know",
            ));
        }
    }
    if let Some(features) = fields.get("features")
        && features
            .as_object()
            .is_none_or(|features| !features.is_empty())
    {
        return Err(EnvironmentError::unsupported(
            "features",
            "glave cannot install features; build them into the image",
        ));
    }
    let image = match fields.get("image") {
        Some(Value::String(name)) => {
            Image::named(name).ok_or_else(|| EnvironmentError::invalid("the image is empty"))?
        }
        Some(_) => return Err(EnvironmentError::invalid("\"image\" must be text")),
        None => return Err(EnvironmentError::invalid("it names no \"image\"")),
    };
    let mut setup = Vec::new();
    for key in SETUP {
        if let Some(command) = fields.get(key) {
            setup.extend(commands(key, command)?);
        }
    }
    let environment = Environment {
        image,
        setup,
        size: size(fields.get("hostRequirements"))?,
        env: variables(&fields)?,
        ..Environment::default()
    };
    check(&environment)?;
    Ok((environment, ignored))
}

fn is_honoured(key: &str) -> bool {
    SETUP.contains(&key)
        || matches!(
            key,
            "image" | "features" | "containerEnv" | "remoteEnv" | "hostRequirements"
        )
}

/// A lifecycle command: text for the shell, a program and its arguments, or several named
/// commands (run in name order).
fn commands(key: &str, command: &Value) -> Result<Vec<String>, EnvironmentError> {
    match command {
        Value::String(text) if !text.trim().is_empty() => Ok(vec![text.clone()]),
        Value::Array(words) => {
            let words: Option<Vec<&str>> = words.iter().map(Value::as_str).collect();
            match words {
                Some(words) if !words.is_empty() => Ok(vec![
                    words
                        .iter()
                        .map(|word| quote(word))
                        .collect::<Vec<_>>()
                        .join(" "),
                ]),
                _ => Err(EnvironmentError::invalid(format!(
                    "{key} must list a program and its arguments as text"
                ))),
            }
        }
        Value::Object(named) => {
            let mut all = Vec::new();
            for command in named.values() {
                all.extend(commands(key, command)?);
            }
            Ok(all)
        }
        _ => Err(EnvironmentError::invalid(format!(
            "{key} must be a command, a list of words, or named commands"
        ))),
    }
}

/// One word for the shell, quoted only when it needs it.
fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// containerEnv, then remoteEnv over it. Values are used as written, so `${...}`
/// substitutions, which only an editor can fill in, are refused.
fn variables(fields: &Map<String, Value>) -> Result<BTreeMap<String, String>, EnvironmentError> {
    let mut env = BTreeMap::new();
    for key in ["containerEnv", "remoteEnv"] {
        let Some(found) = fields.get(key) else {
            continue;
        };
        let Some(map) = found.as_object() else {
            return Err(EnvironmentError::invalid(format!(
                "{key} must be an object"
            )));
        };
        for (name, value) in map {
            let Some(value) = value.as_str() else {
                return Err(EnvironmentError::invalid(format!(
                    "{key}.{name} must be text"
                )));
            };
            if value.contains("${") {
                return Err(EnvironmentError::unsupported(
                    &format!("{key}.{name}"),
                    "variable substitution needs the editor's machine; write the value itself",
                ));
            }
            env.insert(name.clone(), value.to_owned());
        }
    }
    Ok(env)
}

/// hostRequirements: cpus, memory and storage, such as `"8gb"`. GPUs are refused.
fn size(requirements: Option<&Value>) -> Result<Size, EnvironmentError> {
    let Some(requirements) = requirements else {
        return Ok(Size::default());
    };
    let Some(fields) = requirements.as_object() else {
        return Err(EnvironmentError::invalid(
            "hostRequirements must be an object",
        ));
    };
    let mut size = Size::default();
    for (key, value) in fields {
        match key.as_str() {
            "cpus" => {
                size.cpus = Some(
                    value
                        .as_u64()
                        .and_then(|cpus| u32::try_from(cpus).ok())
                        .ok_or_else(|| EnvironmentError::invalid("cpus must be a whole number"))?,
                );
            }
            "memory" => size.memory_gb = Some(gigabytes("memory", value)?),
            "storage" => size.disk_gb = Some(gigabytes("storage", value)?),
            "gpu" => {
                return Err(EnvironmentError::unsupported(
                    "hostRequirements.gpu",
                    "computers with GPUs come later",
                ));
            }
            other => {
                return Err(EnvironmentError::unsupported(
                    &format!("hostRequirements.{other}"),
                    "glave does not know this requirement",
                ));
            }
        }
    }
    Ok(size)
}

/// `"8gb"`, `"512mb"` or `"1tb"`, rounded up to whole gigabytes.
pub(crate) fn gigabytes(key: &str, value: &Value) -> Result<u32, EnvironmentError> {
    let wrong = || EnvironmentError::invalid(format!("{key} must be a size such as \"8gb\""));
    let text = value
        .as_str()
        .ok_or_else(wrong)?
        .trim()
        .to_ascii_lowercase();
    let digits = text.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let unit = text.get(digits.len()..).unwrap_or_default();
    let number: u64 = digits.trim().parse().map_err(|_| wrong())?;
    let megabytes = match unit {
        "tb" => number.checked_mul(1024 * 1024),
        "gb" => number.checked_mul(1024),
        "mb" => Some(number),
        "kb" => Some(number.div_ceil(1024)),
        _ => None,
    }
    .ok_or_else(wrong)?;
    u32::try_from(megabytes.div_ceil(1024)).map_err(|_| wrong())
}
