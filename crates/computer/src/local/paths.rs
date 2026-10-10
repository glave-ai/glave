//! Paths inside a workspace: every path a caller gives is resolved and confined to the
//! workspace root. Symlinks and `..` are followed before the check, so neither can escape.
//! Refusals are whole sentences, written to be shown to the model as they are.

use std::path::{Component, Path, PathBuf};

use glave_ports::ComputerError;

/// An existing folder, made canonical.
pub(crate) async fn folder(path: &str) -> Result<PathBuf, ComputerError> {
    let root = tokio::fs::canonicalize(path)
        .await
        .map_err(|error| ComputerError::NotFound(format!("{path}: {error}")))?;
    let metadata = tokio::fs::metadata(&root)
        .await
        .map_err(|error| ComputerError::NotFound(format!("{path}: {error}")))?;
    if metadata.is_dir() {
        Ok(root)
    } else {
        Err(ComputerError::Refused(format!("{path} is not a folder")))
    }
}

/// Resolves `path` (relative to `root`, or absolute inside it) to a path inside `root`.
pub(crate) async fn resolve(root: &Path, path: &str) -> Result<PathBuf, ComputerError> {
    if path.trim().is_empty() {
        return Err(ComputerError::Refused("The path is empty.".into()));
    }
    let candidate = root.join(path);
    let mut existing = candidate.as_path();
    let mut tail = Vec::new();
    while !tokio::fs::try_exists(existing).await.unwrap_or(false) {
        // A link to something that does not exist yet: writing through it would create its
        // target, wherever that is, so it cannot be checked. Refuse it.
        if tokio::fs::symlink_metadata(existing).await.is_ok() {
            return Err(ComputerError::Refused(format!(
                "{path} goes through a link to something that does not exist, so it cannot be \
                 checked to stay inside the workspace."
            )));
        }
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return Err(ComputerError::Refused(format!(
                "{path} cannot be resolved."
            )));
        };
        tail.push(name.to_owned());
        existing = parent;
    }
    let mut resolved = tokio::fs::canonicalize(existing)
        .await
        .map_err(|error| ComputerError::Failed(format!("{path} cannot be resolved: {error}")))?;
    for name in tail.iter().rev() {
        if !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        ) {
            return Err(ComputerError::Refused(format!(
                "{path} uses '..' or '.' through a folder that does not exist."
            )));
        }
        resolved.push(name);
    }
    if resolved.starts_with(root) {
        Ok(resolved)
    } else {
        Err(ComputerError::Refused(format!(
            "{path} is outside the workspace ({}).",
            root.display()
        )))
    }
}

/// `path` relative to `root`, with `/` between names; `.` for the root itself.
pub(crate) fn relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let names: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if names.is_empty() {
        ".".into()
    } else {
        names.join("/")
    }
}
