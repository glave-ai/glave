//! What untrusted content takes from the rest of its thread (`specs/approvals.md`, "Untrusted
//! content"): its sandboxed commands lose the network, which a command may still ask the
//! person for, and `web_fetch` asks before each page, since the address it fetches could carry
//! what the thread has read. Before the thread reads anything untrusted, nothing changes; Full
//! access, which has no sandbox and asks no one, is left as it is.

use std::sync::Arc;

use glave_event::{ApprovalRequested, Risk, WebPage};
use glave_ports::computer::Sandbox;
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};

/// The sandbox a thread's commands run in: `sandbox`, without the network once the thread has
/// read untrusted content.
pub(crate) fn offline_after(sandbox: Option<Sandbox>, untrusted: bool) -> Option<Sandbox> {
    sandbox.map(|sandbox| Sandbox {
        network: sandbox.network && !untrusted,
        ..sandbox
    })
}

/// The caution on a call of the tool named `name` in `thread`: `web_fetch` after untrusted
/// content, unless the person allowed that for the rest of the thread when asked about it (an
/// answer for `web_fetch` given before does not count). Known by its name, the first source
/// offering a tool keeping it, so glave's own unless it offers none. Only the person clears
/// it: the judge sees the address, never what the thread read.
pub(crate) fn fetch_caution(thread: &Thread, name: &str) -> Option<Risk> {
    let allowed = thread.allowed.contains(&format!("{name}{AFTER}"));
    (name == WebPage::TOOL && thread.untrusted && !allowed).then_some(Risk::FetchesAfterUntrusted)
}

/// What an answer for the rest of the thread to a fetch after untrusted content allows, beside
/// the tool's name, so it is told from an answer about the tool alone.
const AFTER: &str = " after untrusted content";

/// What an answer for the rest of the thread to `asked` allows: its tool, or its tool after
/// untrusted content when that is what it asked about.
pub(crate) fn allowance(asked: &ApprovalRequested) -> String {
    let risk = asked
        .reason
        .as_ref()
        .and_then(|reason| reason.risk.as_ref());
    match risk {
        Some(Risk::FetchesAfterUntrusted) => format!("{}{AFTER}", asked.tool),
        _ => asked.tool.clone(),
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Takes the network from the commands `thread` runs from now on, once it has read untrusted
    /// content and they still have it, in the sandbox otherwise as before; a command already
    /// running keeps what it started with. Checked wherever its commands may have been given
    /// a workspace anew (a turn's steps, a resumed subagent, a reopened worktree) and as each
    /// result comes in. The thread is marked first: if the sandbox then fails to start, the turn
    /// fails rather than run anything with the network.
    pub(crate) async fn offline_if_untrusted(
        &self,
        thread: &mut Thread,
    ) -> Result<(), HarnessError> {
        let workspace = self.workspace_for(thread);
        let sandbox = workspace.sandbox().cloned();
        let offline = offline_after(sandbox.clone(), thread.untrusted);
        if offline != sandbox {
            let within = workspace.within(offline.as_ref()).await?;
            thread.workspace = Some(Arc::new(within));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::computer::SandboxLevel;

    use super::*;

    #[test]
    fn untrusted_content_takes_only_the_network() {
        let sandbox = |network| Sandbox {
            level: SandboxLevel::Project,
            network,
            home: None,
            glave_home: None,
        };
        assert_eq!(
            offline_after(Some(sandbox(true)), false),
            Some(sandbox(true))
        );
        assert_eq!(
            offline_after(Some(sandbox(true)), true),
            Some(sandbox(false))
        );
        assert_eq!(
            offline_after(Some(sandbox(false)), true),
            Some(sandbox(false))
        );
        assert_eq!(
            offline_after(None, true),
            None,
            "no sandbox, as in Full access"
        );
    }
}
