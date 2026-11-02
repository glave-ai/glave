//! The sandbox around commands, as the harness sees it: the settings travel with every
//! command to the computer, which enforces them (`specs/sandbox.md`); here they become a note
//! for the model when a command looks blocked, what of the sandbox a request to run one
//! command asks to run without, and a record of what a turn ran under.

use glave_event::{Arguments, Lifted, OutsideSandbox};
use glave_ports::computer::{Sandbox, SandboxLevel};
use glave_ports::{BlobStore, EventLog, ModelProvider, ToolOutput};
use serde::Serialize;

use crate::approval::Approver;
use crate::calls::NOT_IN_PLAN;
use crate::harness::{Harness, Thread};
use crate::tools::Tool;
use crate::tools::workspace::Workspace;
use crate::turn::Live;

/// What the model is told when a command would have the network where commands go without
/// it, and the organization does not let one ask.
pub(crate) const NOT_ALLOWED: &str = "Not run: commands here go without the internet, and \
     asking to run one with it, or outside the sandbox, is not allowed here. Do not try to \
     reach the internet another way, and do not stand in for what it would fetch, such as by \
     writing your own replacement for a package. Finish what does not depend on it, and say at \
     the end which command needs it.";

/// What the model is told when a command would leave the sandbox in plan mode.
pub(crate) const PLAN_STAYS: &str = "Not run: in plan mode commands stay in the sandbox, where \
     they change nothing, until the person approves the plan. Look around with commands that \
     only read, and put what needs more in the plan.";

/// What the model is told when what a command would run without of the sandbox changed while
/// it waited for the person, so their answer was about something else.
pub(crate) const CHANGED: &str = "Not run: the sandbox changed while this waited for the \
     person's answer, so the answer no longer fits it. Run the command again if it is still \
     needed.";

/// What a command may ask the person for beyond the sandbox it runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asks {
    /// To run outside it, where the sandbox has the network, or there is none.
    Outside,
    /// To run outside it, or in it with the network, where it has none.
    OutsideOrNetwork,
    /// Nothing: the sandbox has no network, and the policy forbids asking for it, so running
    /// outside the sandbox, which has it, too.
    Nothing,
    /// Nothing, in plan mode: what a command runs in changes nothing, and it stays so.
    Never,
}

impl Asks {
    /// What commands in `workspace` may ask for.
    pub(crate) fn of(workspace: &Workspace) -> Self {
        if workspace.plans() {
            return Self::Never;
        }
        match workspace.sandbox() {
            Some(sandbox) if !sandbox.network && workspace.asks_network() => Self::OutsideOrNetwork,
            Some(sandbox) if !sandbox.network => Self::Nothing,
            _ => Self::Outside,
        }
    }
}

/// What of the sandbox a call of the built-in `bash` asks to run without, when commands in
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

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The result the model is given instead of running a call that would run without `lifted`
    /// of the sandbox `thread`'s commands run in, where the person is asked, or was, about
    /// `asked`: refused when it would leave the sandbox otherwise than they were asked, since
    /// the sandbox changed while it waited, and when it would leave it at all where its
    /// commands may not ask to. `None` when it may go on.
    pub(crate) fn let_out_refused(
        &self,
        thread: &Thread,
        lifted_asked: (Option<Lifted>, Option<Lifted>),
        summary: &str,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Option<ToolOutput> {
        let_out_refused(self.workspace_for(thread), lifted_asked, summary, live)
    }
}

/// [`Harness::let_out_refused`] for the sandbox `workspace` runs commands in.
fn let_out_refused(
    workspace: &Workspace,
    (lifted, asked): (Option<Lifted>, Option<Lifted>),
    summary: &str,
    live: &mut (dyn FnMut(Live) + Send),
) -> Option<ToolOutput> {
    let asks = Asks::of(workspace);
    let (refused, why) = if lifted.is_some() && lifted != asked {
        (CHANGED, "not run: the sandbox changed")
    } else if lifted.is_some() && asks == Asks::Never {
        (PLAN_STAYS, NOT_IN_PLAN)
    } else if lifted.is_some() && asks == Asks::Nothing {
        (NOT_ALLOWED, "not allowed here")
    } else {
        return None;
    };
    live(Live::ToolFinished {
        summary: format!("{summary} ({why})"),
        is_error: true,
    });
    Some(ToolOutput::text(true, refused))
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

/// A note for the model when a failed command in a sandbox looks blocked by it, so it stops
/// guessing: it can ask the person for what `asks` allows (to run that command outside the
/// sandbox, or with the network where the sandbox has none), or do the task another way.
/// Follows Codex's heuristic: a failure other than usage (2), not executable (126) or not found
/// (127) that mentions a refusal, or, without the network, a name or address it could not
/// reach.
pub(crate) fn blocked_note(asks: Asks, exit: Option<i32>, output: &str) -> Option<&'static str> {
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
    let offline: &[&str] = match asks {
        Asks::Outside | Asks::Never => &[],
        Asks::OutsideOrNetwork | Asks::Nothing => &[
            "name resolution",
            "getaddrinfo",
            "enotfound",
            "network is unreachable",
        ],
    };
    let refused = refusals
        .iter()
        .chain(offline)
        .any(|sign| output.contains(sign));
    refused.then_some(match asks {
        Asks::Outside => {
            "[This command may have been blocked by the sandbox: commands here cannot write \
             outside the project, change git hooks or config, or read secrets (credential \
             stores and files such as .env and private keys are hidden). If the task needs \
             this very command, run it again with outside_sandbox: true and a reason, and the \
             person is asked to allow it; otherwise do the task another way or tell the person \
             what is needed.]"
        }
        Asks::OutsideOrNetwork => {
            "[This command may have been blocked by the sandbox: commands here cannot use the \
             network, write outside the project, change git hooks or config, or read secrets \
             (credential stores and files such as .env and private keys are hidden). If the \
             task needs this very command, run it again with a reason and network: true when \
             it needs the internet, or outside_sandbox: true when it needs more, and the person \
             is asked to allow it; otherwise do the task another way or tell the person what is \
             needed.]"
        }
        Asks::Nothing => {
            "[This command may have been blocked by the sandbox: commands here cannot use the \
             network, write outside the project, change git hooks or config, or read secrets \
             (credential stores and files such as .env and private keys are hidden), and none \
             may ask to run with the network or outside the sandbox. Do the task another way, \
             or tell the person what is needed.]"
        }
        Asks::Never => {
            "[This command may have been blocked by the sandbox: in plan mode commands cannot \
             change any file or read secrets, and none may leave the sandbox. Look around with \
             commands that only read, and put what needs changing in the plan.]"
        }
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
