//! Why a process could not be started or recorded, or did not finish in the time it was given.

use std::path::PathBuf;
use std::time::Duration;

/// Why a process could not be started, recorded once it was, or waited for to its end.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// The operating system would not start it.
    #[error("could not start {program}: {source}")]
    Start {
        program: String,
        source: std::io::Error,
    },
    /// It was still running when the time its caller gave it ran out, so it was stopped.
    #[error("{program} did not finish within {}", glave_ports::spoken(*limit))]
    TimedOut { program: String, limit: Duration },
    /// The ledger already holds as many processes as it may.
    #[error("glave already runs {0} processes on this computer; stop some first")]
    Full(usize),
    /// The helper that stops what glave started when glave ends could not be started.
    #[error(
        "could not start glave's helper that stops what glave starts when it ends ({helper}): {source}"
    )]
    Lifeline {
        helper: PathBuf,
        source: std::io::Error,
    },
    /// The ledger could not be read or written.
    #[error("could not record a process glave started in {path}: {source}")]
    Ledger {
        path: PathBuf,
        source: std::io::Error,
    },
}
