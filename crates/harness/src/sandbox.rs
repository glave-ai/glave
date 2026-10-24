//! The sandbox around commands, as the harness sees it: the settings travel with every
//! command to the computer, which enforces them (`specs/sandbox.md`); here they become a note
//! for the model when a command looks blocked, the caution on a request to run one command
//! outside the sandbox or with its network, and a record of what a turn ran under.

use glave_event::{Arguments, Lifted, OutsideSandbox};
use glave_ports::computer::{Sandbox, SandboxLevel};
use serde::Serialize;

use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// Why running a command outside the sandbox needs the person, as the model and the judge are
/// told it; the person reads it as a sentence.
pub(crate) const OUTSIDE_SANDBOX: &str =
    "it runs outside the sandbox, where it can read and change any file you can";

/// Why running a command with the network needs the person, where the sandbox has none.
pub(crate) const WITH_NETWORK: &str =
    "it can reach the internet, which commands here otherwise cannot";

/// What the model is told when a command would have the network where commands go without
/// it, and the organization does not let one ask.
pub(crate) const NOT_ALLOWED: &str = "Not run: commands here go without the internet, and \
     asking to run one with it, or outside the sandbox, is not allowed here. Do not try to \
     reach the internet another way. Finish what you can without it, and say at the end which \
     command needs it.";

/// What a call of the built-in `bash` asks to run without of the sandbox, when commands in
/// `workspace` run in one: all of it, or its network where it has none. Where they run
/// without one, or already have the network, the request changes nothing, so there is
/// nothing to ask.
pub(crate) fn outside(
    workspace: &Workspace,
    name: &str,
    arguments: &Arguments,
) -> Option<OutsideSandbox> {
    if Tool::named(name) != Some(Tool::Bash) {
        return None;
    }
    let sandbox = workspace.sandbox()?;
    let asked = crate::tools::outside_sandbox(arguments)?;
    (asked.lifted == Lifted::Sandbox || !sandbox.network).then_some(asked)
}

/// Whether a request to run without the sandbox's limits may not even ask: commands in
/// `workspace` go without the network, which it would have, and `ask_network` is off.
pub(crate) fn forbidden(workspace: &Workspace, ask_network: bool) -> bool {
    !ask_network && workspace.sandbox().is_some_and(|sandbox| !sandbox.network)
}

/// The reason a call needs the person: the command's own caution, if it has one, and then
/// running outside the sandbox or with its network, when it would.
pub(crate) fn with_outside(caution: Option<String>, lifted: Option<Lifted>) -> Option<String> {
    let lifted = match lifted {
        Some(Lifted::Sandbox) => OUTSIDE_SANDBOX,
        Some(Lifted::Network) => WITH_NETWORK,
        None => return caution,
    };
    Some(match caution {
        Some(caution) => format!("{caution}, and {lifted}"),
        None => lifted.to_owned(),
    })
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
/// guessing: it can ask the person to run that command outside the sandbox, or with the
/// network where the sandbox has none, or do the task another way. Follows Codex's heuristic:
/// a failure other than usage (2), not executable (126) or not found (127) that mentions a
/// refusal, or, without the network, a name or address it could not reach.
pub(crate) fn blocked_note(
    sandbox: &Sandbox,
    exit: Option<i32>,
    output: &str,
) -> Option<&'static str> {
    if matches!(exit, Some(0 | 2 | 126 | 127)) {
        return None;
    }
    let output = output.to_ascii_lowercase();
    let refusals = [
        "operation not permitted",
        "permission denied",
        "read-only file system",
        "sandbox",
        "could not resolve host",
        "nodename nor servname",
    ];
    let offline: &[&str] = if sandbox.network {
        &[]
    } else {
        &[
            "name resolution",
            "getaddrinfo",
            "enotfound",
            "network is unreachable",
        ]
    };
    let refused = refusals
        .iter()
        .chain(offline)
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
         task needs this very command, run it again with a reason and network: true when \
         it needs the internet, or outside_sandbox: true when it needs more, and the person \
         is asked to allow it; otherwise do the task another way or tell the person what is \
         needed.]"
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
