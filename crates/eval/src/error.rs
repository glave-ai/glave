//! Why an eval could not run. A task that runs and fails is not an error: it is a result.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvalError {
    #[error("could not {action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("task {}: {message}", dir.display())]
    Task { dir: PathBuf, message: String },
    #[error("judge case {}:{line}: {message}", file.display())]
    Case {
        file: PathBuf,
        line: usize,
        message: String,
    },
    #[error("{0}")]
    Setup(String),
}

impl EvalError {
    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
    ) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io {
            action,
            path,
            source,
        }
    }
}
