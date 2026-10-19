//! Where a command's requests go (`specs/one-server.md`, "The CLI as a client"): to the
//! person's server over its socket, started first from the person's own terminal when it does
//! not run; and where none runs and no one is at the terminal, to a server in the command's own
//! process, answered by no one, so a script leaves nothing behind.

use std::path::Path;

use anyhow::bail;
use glave_ports::ProjectId;
use glave_session::protocol::{NewFolder, Reply, Request};

use crate::socket::Connection;
use crate::wiring::Wired;

/// The server a command's requests go to.
pub enum Reached {
    /// The person's server.
    Server(Connection),
    /// A server in this process.
    Here(Box<Wired>),
}

/// Runs `run` with the server [`Reached::open`] finds, and lets go of it after, however `run`
/// ends.
pub async fn with<T>(
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
    pub async fn open() -> anyhow::Result<Self> {
        Ok(match crate::server::connect_or_start().await? {
            Some(server) => Self::Server(server),
            None => Self::Here(Box::new(crate::wiring::here().await?)),
        })
    }

    /// Sends `request` and answers with the reply, or the error the server gave.
    pub async fn request(&mut self, request: Request) -> anyhow::Result<Reply> {
        match self {
            Self::Server(server) => server.request(request).await,
            Self::Here(wired) => Box::pin(wired.server.request(request))
                .await
                .map_err(anyhow::Error::new),
        }
    }

    /// The project `folder` leads to, made the first time, with `folder` as its folder: where
    /// its tasks and schedules' runs make their copies.
    pub async fn project(&mut self, folder: &Path) -> anyhow::Result<ProjectId> {
        let name = folder.file_name().map_or_else(
            || folder.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let path = folder.to_string_lossy().into_owned();
        let created = Request::ProjectCreate {
            name,
            members: Vec::new(),
            folder: Some(NewFolder::Path { path }),
        };
        let Reply::Project { project, .. } = self.request(created).await? else {
            bail!("glave answered project/create with something else");
        };
        Ok(project.id)
    }

    /// Lets go of a server in this process; a connection to the person's server just closes.
    pub async fn close(self) {
        if let Self::Here(wired) = self {
            wired.server.shutdown().await;
            wired.close().await;
        }
    }
}
