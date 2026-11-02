//! What untrusted content takes from the rest of its thread (`specs/approvals.md`, "Untrusted
//! content"): its sandboxed commands lose the network, which a command may still ask the
//! person for, and `web_fetch` asks before each page, since the address it fetches could carry
//! what the thread has read. Before the thread reads anything untrusted, nothing changes; Full
//! access, which has no sandbox and asks no one, is left as it is.

use std::sync::Arc;

use glave_event::{Risk, WebPage};
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
/// content, unless the person allowed it for the rest of the thread. Known by its name, the
/// first source offering a tool keeping it, so glave's own unless it offers none. Only the
/// person clears it: the judge sees the address, never what the thread read.
pub(crate) fn fetch_caution(thread: &Thread, name: &str) -> Option<Risk> {
    (name == WebPage::TOOL && thread.untrusted && !thread.allowed.contains(name))
        .then_some(Risk::FetchesAfterUntrusted)
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Takes in whether a call's result was untrusted: the first such result in `thread`
    /// takes the network from the commands it runs from now on, in the sandbox otherwise as
    /// before. A command already running keeps what it started with.
    pub(crate) async fn took_in(
        &self,
        thread: &mut Thread,
        untrusted: bool,
    ) -> Result<(), HarnessError> {
        if !untrusted || thread.untrusted {
            return Ok(());
        }
        thread.untrusted = true;
        let workspace = self.workspace_for(thread);
        let sandbox = workspace.sandbox().cloned();
        let offline = offline_after(sandbox.clone(), true);
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
