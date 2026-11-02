//! Where a command's requests go (`specs/one-server.md`, "The CLI as a client"): to the
//! person's server over its socket, started first from the person's own terminal when it does
//! not run; and where none runs and no one is at the terminal, to a server in the command's own
//! process, answered by no one, so a script leaves nothing behind.

use glave_session::protocol::{Reply, Request};

use crate::client::Session;
use crate::socket::Connection;
use crate::wiring::Wired;

/// The server a command's requests go to.
pub(crate) enum Reached {
    /// The person's server.
    Server(Connection),
    /// A server in this process.
    Here(Box<Wired>),
}

/// Runs `run` with the server [`Reached::open`] finds, and lets go of it after, however `run`
/// ends.
pub(crate) async fn with<T>(
    run: impl AsyncFnOnce(&mut Reached) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut reached = Reached::open().await?;
    let ran = run(&mut reached).await;
    reached.close().await;
    ran
}

impl Reached {
    /// The person's server, started first from the person's own terminal; a server in this
    /// process where none runs and no one is at the terminal.
    pub(crate) async fn open() -> anyhow::Result<Self> {
        Ok(match crate::server::connect_or_start().await? {
            Some(server) => Self::Server(server),
            // Boxed: wiring a server here is a large future, kept off every caller's.
            None => Self::Here(Box::new(Box::pin(crate::wiring::here()).await?)),
        })
    }

    /// The server reached, as a turn's requests reach it.
    pub(crate) fn session(&self) -> Session<'_> {
        match self {
            Self::Server(server) => Session::Elsewhere(server.remote()),
            Self::Here(wired) => Session::Here(&wired.server),
        }
    }

    /// Sends `request` and answers with the reply, or the error the server gave.
    pub(crate) async fn request(&self, request: Request) -> anyhow::Result<Reply> {
        self.session()
            .request(request)
            .await
            .map_err(anyhow::Error::new)
    }

    /// Lets go of a server in this process; a connection to the person's server just closes.
    pub(crate) async fn close(self) {
        if let Self::Here(wired) = self {
            wired.server.shutdown().await;
            wired.close().await;
        }
    }
}
