//! A process glave started to run on its own, owned by whoever holds it: it leads a process
//! group of its own, and when this drops, everything still in that group is killed, its ledger
//! entry removed and the lifeline's helper told, so nothing it started outlives its owner.

use std::ops::{Deref, DerefMut};

use rustix::process::{Pid, Signal};

use crate::group::{self, GRACE};
use crate::ledger::Entry;
use crate::lifeline::Tie;

/// A running process and the group it leads. It derefs to tokio's child for its pipes; waiting
/// for its exit goes through its own [`Child::wait`] and [`Child::try_wait`]; stopping it, and
/// dropping it, reach its whole group.
#[derive(Debug)]
#[must_use = "dropping it stops the process and everything it started"]
pub struct Child {
    inner: tokio::process::Child,
    group: Option<Pid>,
    /// Its ledger entry, when it has one; removed once the group is gone.
    entry: Option<Entry>,
    /// Its tie to glave, when glave has a lifeline: held while the leader is glave's unreaped
    /// child, so its number, and its group's, cannot be another's; let go once the leader is
    /// reaped and the group is no longer stopped by glave, or once the group is gone.
    tie: Option<Tie>,
}

impl Child {
    pub(crate) fn new(inner: tokio::process::Child) -> Self {
        let group = inner
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(group::of);
        Self {
            inner,
            group,
            entry: None,
            tie: None,
        }
    }

    /// The same child, tied to glave by the lifeline.
    pub(crate) fn tied(mut self, tie: Tie) -> Self {
        self.tie = Some(tie);
        self
    }

    /// The same child, with its ledger entry.
    pub(crate) fn recorded(mut self, entry: Option<Entry>) -> Self {
        self.entry = entry;
        self
    }

    /// Waits for it to exit, as tokio's does. Once it has, its number is free to be given to
    /// another process, so the lifeline lets its group go, as the reaper leaves a group whose
    /// leader ended: what it left running is stopped by whoever holds this
    /// ([`Child::stop_left`], or dropping it).
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.inner.wait().await?;
        self.tie = None;
        Ok(status)
    }

    /// Whether it has exited, as tokio's says; once it has, as [`Child::wait`].
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let status = self.inner.try_wait()?;
        if status.is_some() {
            self.tie = None;
        }
        Ok(status)
    }

    /// Stops it and everything it started: SIGTERM to its group, then SIGKILL to whatever is
    /// still there after a grace period. Returns once the group is empty, so a child that
    /// outlives the leader, or ignores SIGTERM, is stopped too.
    pub async fn stop(&mut self) {
        if let Some(group) = self.group {
            group::signal(group, Signal::TERM);
            if tokio::time::timeout(GRACE, self.gone(group)).await.is_err() {
                group::signal(group, Signal::KILL);
                // SIGKILL cannot be ignored; this waits only for the kernel to finish the job.
                let _ = tokio::time::timeout(GRACE, self.gone(group)).await;
            }
        }
        let _ = self.inner.kill().await;
        self.entry = None;
        self.tie = None;
    }

    /// Stops what is left of its group once it has exited: what it left running.
    pub async fn stop_left(&mut self) {
        if let Some(group) = self.group {
            group::stop_all(&[group]).await;
        }
        self.entry = None;
        self.tie = None;
    }

    /// Resolves once the leader has exited and nothing is left in its group, not even a
    /// process waiting to be reaped. glave's workers run with an init that reaps orphans
    /// (Docker's `Init`, the Pod's shared process namespace), as does any desktop.
    async fn gone(&mut self, group: Pid) {
        let _ = self.inner.wait().await;
        group::emptied(group).await;
    }
}

impl Deref for Child {
    type Target = tokio::process::Child;

    fn deref(&self) -> &tokio::process::Child {
        &self.inner
    }
}

impl DerefMut for Child {
    fn deref_mut(&mut self) -> &mut tokio::process::Child {
        &mut self.inner
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // While anything is left in the group its number is not given to another, so this
        // reaches only what this process started. tokio reaps the leader (`kill_on_drop`).
        if let Some(group) = self.group
            && group::exists(group)
        {
            group::signal(group, Signal::KILL);
        }
    }
}
