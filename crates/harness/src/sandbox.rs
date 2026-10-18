//! The sandbox around commands, as the harness sees it: the settings travel with every
//! command to the computer, which enforces them (`specs/sandbox.md`); here they become a note
//! for the model when a command looks blocked, the caution on a request to run one command
//! outside the sandbox, and a record of what a turn ran under.

use glave_event::{Arguments, OutsideSandbox};
use glave_ports::computer::{Sandbox, SandboxLevel};
use serde::Serialize;

use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// Why running a command outside the sandbox needs the person, as the model and the judge are
/// told it; the person reads it as a sentence.
pub(crate) const OUTSIDE_SANDBOX: &str =
    "it runs outside the sandbox, where it can read and change any file you can";

/// What a call of the built-in `bash` asks to run outside the sandbox, when commands in
/// `workspace` run in one. Where they run without one, the request changes nothing, so there
/// is nothing to ask.
pub(crate) fn outside(
    workspace: &Workspace,
    name: &str,
    arguments: &Arguments,
) -> Option<OutsideSandbox> {
    let bash = Tool::named(name) == Some(Tool::Bash);
    (bash && workspace.sandbox().is_some())
        .then(|| crate::tools::outside_sandbox(arguments))
        .flatten()
}

/// The reason a call needs the person: the command's own caution, if it has one, and then
/// running outside the sandbox, when it would.
pub(crate) fn with_outside(caution: Option<String>, outside: bool) -> Option<String> {
    match (caution, outside) {
        (Some(caution), true) => Some(format!("{caution}, and {OUTSIDE_SANDBOX}")),
        (None, true) => Some(OUTSIDE_SANDBOX.to_owned()),
        (caution, false) => caution,
    }
}

/// What the thread records about the protections a turn ran under.
pub(crate) fn recorded(sandbox: &Sandbox) -> Recorded {
    Recorded {
        level: sandbox.level,
        network: sandbox.network,
        enforcement: ENFORCEMENT,
    }
}

/// How this platform enforces the sandbox: Seatbelt on macOS; on Linux bubblewrap and
/// seccomp, or Landlock and seccomp at `read-only` where bubblewrap cannot start.
const ENFORCEMENT: &str = if cfg!(target_os = "macos") {
    "seatbelt"
} else if cfg!(target_os = "linux") {
    "linux"
} else {
    "none"
};

/// A note for the model when a failed command looks blocked by the sandbox, so it stops
/// guessing: it can ask the person to run that command outside the sandbox, or do the task
/// another way. Follows Codex's heuristic: a failure other than usage (2), not executable
/// (126) or not found (127) that mentions a refusal.
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
         outside the project, change git hooks or config, or read secrets (credential \
         stores and files such as .env and private keys are hidden). If the task needs this \
         very command, run it again with outside_sandbox: true and a reason, and the person \
         is asked to allow it; otherwise do the task another way or tell the person what is \
         needed.]"
    } else {
        "[This command may have been blocked by the sandbox: commands here cannot use the \
         network, write outside the project, change git hooks or config, or read secrets \
         (credential stores and files such as .env and private keys are hidden). If the \
         task needs this very command, run it again with outside_sandbox: true and a \
         reason, and the person is asked to allow it; otherwise do the task another way or \
         tell the person what is needed.]"
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
