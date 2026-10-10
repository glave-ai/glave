//! The opt-in sandbox around commands, as the harness sees it: the settings travel with every
//! command to the computer, which enforces them (`specs/sandbox.md`); here they become a note
//! for the model when a command looks blocked, and a record of what a turn ran under.

use glave_ports::{Sandbox, SandboxLevel};
use serde::Serialize;

/// What the thread records about the protections a turn ran under.
pub(crate) fn recorded(sandbox: &Sandbox) -> Recorded {
    Recorded {
        level: sandbox.level,
        network: sandbox.network,
        enforcement: "seatbelt",
    }
}

/// A note for the model when a failed command looks blocked by the sandbox, so it stops trying
/// instead of guessing. Follows Codex's heuristic: a failure other than usage (2), not
/// executable (126) or not found (127) that mentions a refusal.
pub(crate) fn blocked_note(
    sandbox: &Sandbox,
    exit: Option<i32>,
    output: &str,
) -> Option<&'static str> {
    if matches!(exit, Some(0 | 2 | 126 | 127)) {
        return None;
    }
    let output = output.to_ascii_lowercase();
    let refused = [
        "operation not permitted",
        "permission denied",
        "read-only file system",
        "sandbox",
        "could not resolve host",
        "nodename nor servname",
    ]
    .iter()
    .any(|sign| output.contains(sign));
    refused.then_some(if sandbox.network {
        "[This command may have been blocked by the sandbox: commands here cannot write \
         outside the project, change git hooks or config, or read secrets. That cannot \
         be lifted from inside; do the task another way or tell the person what is needed.]"
    } else {
        "[This command may have been blocked by the sandbox: commands here cannot use the \
         network, write outside the project, change git hooks or config, or read secrets. \
         That cannot be lifted from inside; do the task another way or tell the person \
         what is needed.]"
    })
}

/// The sandbox as recorded in a turn's configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Recorded {
    level: SandboxLevel,
    network: bool,
    enforcement: &'static str,
}

#[cfg(test)]
mod tests;
