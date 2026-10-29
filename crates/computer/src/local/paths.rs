//! Workspace folders and paths shown relative to them. Paths a caller gives are resolved
//! and confined in `anchored`.

use std::path::{Path, PathBuf};

use glave_ports::computer::{ComputerError, Refusal};

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
        Err(ComputerError::Refused(
            Refusal::NotAFolder {
                path: path.to_owned(),
            }
            .into(),
        ))
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
