//! A person's terminals on a computer that may be asleep (`specs/computers.md`, "Terminals"):
//! one opens on the computer woken for it, as a command wakes it; every other call finds the
//! computer as it is awake now, asked without waking it ([`Peek`]), never by what this
//! [`Waking`] saw before, which a server restarted, or another server, never saw. A computer
//! asleep keeps no processes, so it has none and is not woken to say so; one this server can't
//! reach says so, never that it has none.

use std::sync::PoisonError;

use glave_ports::computer::{ComputerError, ComputerId, TerminalId, TerminalInfo, TerminalSize};

#[cfg(doc)]
use super::Peek;
use super::{Real, Waking};

impl Waking {
    /// Opens a terminal at `folder` for `owner` on the computer, woken for it, with the
    /// workspace planned there opened for real first, as its first command would open it: a
    /// chat's scratch folder is made only then.
    pub(super) async fn open_terminal_woken(
        &self,
        computer: ComputerId,
        (folder, owner): (&str, &str),
        size: TerminalSize,
    ) -> Result<TerminalId, ComputerError> {
        if computer != self.home.computer {
            return Err(ComputerError::NotFound(format!("no computer {computer}")));
        }
        let planned = self
            .workspaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .find(|planned| planned.info.root == folder)
            .map_or(self.home.id, |planned| planned.info.id);
        let home = self.real(planned).await?;
        let opened = home
            .computers
            .open_terminal(home.computer, folder, owner, size);
        self.done(opened.await)
    }

    /// `owner`'s terminals on the computer awake; none while it sleeps.
    pub(super) async fn terminals_awake(
        &self,
        owner: &str,
    ) -> Result<Vec<TerminalInfo>, ComputerError> {
        match self.awake_now().await? {
            Some(awake) => awake.computers.terminals(awake.computer, owner).await,
            None => Ok(Vec::new()),
        }
    }

    /// The computer awake, where `terminal` is; not found while it sleeps, which ended it.
    pub(super) async fn terminal_awake(&self, terminal: TerminalId) -> Result<Real, ComputerError> {
        self.awake_now()
            .await?
            .ok_or_else(|| ComputerError::NotFound(format!("no terminal {terminal}")))
    }

    /// Its home on the computer awake now, as [`Peek`] finds it; never wakes it.
    async fn awake_now(&self) -> Result<Option<Real>, ComputerError> {
        Ok((self.peek)().await?.map(|awake| Real {
            computers: awake.computers,
            computer: awake.home.computer,
            workspace: awake.home.id,
        }))
    }
}
