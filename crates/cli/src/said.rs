//! Where glave's own sentences about its run go: where it serves, what it set aside, that it is
//! stopping, why it stopped. With the log on standard error (`--log -`), into the log, each at
//! its level, so every line there is JSON for whatever collects it; else onto standard error as
//! the plain `glave: ...` lines a person at a terminal reads.

use std::fmt::Display;
use std::path::Path;

/// Where this run's sentences go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Said {
    /// Onto standard error, as plain lines.
    Plainly,
    /// Into the log, which is on standard error.
    Logged,
}

impl Said {
    /// Where a run that logs to `log` says things.
    pub(crate) fn of(log: Option<&Path>) -> Self {
        if crate::telemetry::on_standard_error(log) {
            Self::Logged
        } else {
            Self::Plainly
        }
    }

    /// `sentence`, news of the run.
    pub(crate) fn info(self, sentence: &str) {
        match self {
            Self::Plainly => eprintln!("glave: {sentence}"),
            Self::Logged => tracing::info!("{sentence}"),
        }
    }

    /// `sentence`, something that went wrong which glave went on from, or that the person who
    /// runs it may want to change.
    pub(crate) fn warn(self, sentence: &str) {
        match self {
            Self::Plainly => eprintln!("glave: {sentence}"),
            Self::Logged => tracing::warn!("{sentence}"),
        }
    }

    /// That `what` failed with `error` and someone must act: `glave: <what>: <error>`, or an
    /// error in the log whose message is `what` alone, so alike failures group, with `error`
    /// beside it.
    pub(crate) fn failed(self, what: &str, error: &dyn Display) {
        match self {
            Self::Plainly => eprintln!("glave: {what}: {error}"),
            Self::Logged => tracing::error!(%error, "{what}"),
        }
    }

    /// The error glave stopped on: as itself, or an error in the log that says glave stopped,
    /// with the error beside it.
    pub(crate) fn stopped(self, error: &anyhow::Error) {
        match self {
            Self::Plainly => eprintln!("glave: {error:#}"),
            Self::Logged => tracing::error!(error = %format!("{error:#}"), "glave stopped"),
        }
    }
}
