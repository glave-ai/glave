//! Reading a project's environment: its `.devcontainer/devcontainer.json` when present
//! (standards first), else its `.glave/environment.toml`, into one typed [`Environment`].

mod devcontainer;
mod jsonc;
mod recipe;

use std::path::{Path, PathBuf};

use thiserror::Error;

pub use devcontainer::parse_devcontainer;
pub use glave_ports::Environment;
pub use recipe::parse_environment_toml;

/// Where devcontainer.json may be, in the order looked for.
const DEVCONTAINER: [&str; 2] = [".devcontainer/devcontainer.json", ".devcontainer.json"];
/// glave's own environment file.
const ENVIRONMENT_TOML: &str = ".glave/environment.toml";

/// Why an environment file could not be used. Nothing it asks for is quietly skipped: what
/// glave cannot honour is refused by name.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnvironmentError {
    #[error("{file}: {message}")]
    Invalid { file: String, message: String },
    #[error("{file}: {key} is not supported: {why}")]
    Unsupported {
        file: String,
        key: String,
        why: String,
    },
    #[error("could not read {file}: {message}")]
    Unreadable { file: String, message: String },
}

/// An environment read from a project, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub environment: Environment,
    pub file: PathBuf,
    /// Settings the file holds that only concern an editor or a port preview, which glave
    /// leaves alone; named so a surface can say so.
    pub ignored: Vec<String>,
}

/// The environment `project` asks for, or `None` when it names none.
pub async fn read_environment(project: &Path) -> Result<Option<Found>, EnvironmentError> {
    for name in DEVCONTAINER {
        let file = project.join(name);
        if let Some(text) = read(&file).await? {
            let (environment, ignored) = parse_devcontainer(&text).map_err(|e| e.at(name))?;
            return Ok(Some(Found {
                environment,
                file,
                ignored,
            }));
        }
    }
    let file = project.join(ENVIRONMENT_TOML);
    match read(&file).await? {
        Some(text) => Ok(Some(Found {
            environment: parse_environment_toml(&text).map_err(|e| e.at(ENVIRONMENT_TOML))?,
            file,
            ignored: Vec::new(),
        })),
        None => Ok(None),
    }
}

async fn read(file: &Path) -> Result<Option<String>, EnvironmentError> {
    match tokio::fs::read_to_string(file).await {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(EnvironmentError::Unreadable {
            file: file.display().to_string(),
            message: error.to_string(),
        }),
    }
}

impl EnvironmentError {
    /// The same error, naming `file`.
    fn at(self, file: &str) -> Self {
        match self {
            Self::Invalid { message, .. } => Self::Invalid {
                file: file.to_owned(),
                message,
            },
            Self::Unsupported { key, why, .. } => Self::Unsupported {
                file: file.to_owned(),
                key,
                why,
            },
            Self::Unreadable { message, .. } => Self::Unreadable {
                file: file.to_owned(),
                message,
            },
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            file: String::new(),
            message: message.into(),
        }
    }

    pub(crate) fn unsupported(key: &str, why: &str) -> Self {
        Self::Unsupported {
            file: String::new(),
            key: key.to_owned(),
            why: why.to_owned(),
        }
    }
}

/// Whether `name` can be an environment variable's name.
pub(crate) fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests;
