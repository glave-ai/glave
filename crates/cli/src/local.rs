//! Telling glave commands apart from the glave that is running: while a `glave` (a session or
//! `glave serve`) runs, it listens on `session.sock` in GLAVE_HOME, and a command that changes
//! what that glave may hold (archiving a thread, cleaning up worktrees) asks it, so every
//! append to a thread goes through the one writer. With nothing listening, the command does
//! the work itself. Only the first glave to start listens; the socket is private to the
//! person, as GLAVE_HOME is.

use std::path::{Path, PathBuf};

use glave_session::protocol::{Reply, Request};

use crate::client::Glave;
use crate::home;

/// The socket's name in GLAVE_HOME.
pub const SOCKET: &str = "session.sock";

/// The most commands served at once.
#[cfg(unix)]
const MOST_PEERS: usize = 16;

fn socket() -> anyhow::Result<PathBuf> {
    Ok(home::folder()?.join(SOCKET))
}

#[cfg(unix)]
pub use unix::{Listening, Peer};

#[cfg(not(unix))]
pub use other::{Listening, Peer};

#[cfg(unix)]
mod unix {
    use tokio::io::BufReader;
    use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::task::{JoinHandle, JoinSet};

    use super::{Glave, MOST_PEERS, Path, PathBuf, Reply, Request, socket};
    use glave_session::protocol::ApiError;

    /// A connection to the glave running for this GLAVE_HOME.
    pub struct Peer {
        input: BufReader<OwnedReadHalf>,
        output: OwnedWriteHalf,
    }

    impl Peer {
        /// Connects to the running glave, or `None` when none is listening.
        pub async fn connect() -> Option<Self> {
            Self::at(&socket().ok()?).await
        }

        async fn at(path: &Path) -> Option<Self> {
            let (input, output) = UnixStream::connect(path).await.ok()?.into_split();
            Some(Self {
                input: BufReader::new(input),
                output,
            })
        }

        /// Asks the running glave one question.
        pub async fn call(&mut self, request: &Request) -> Result<Reply, ApiError> {
            glave_session::rpc::call(&mut self.input, &mut self.output, request).await
        }
    }

    /// This glave listening for commands, until [`Listening::stop`].
    pub struct Listening {
        task: JoinHandle<()>,
        path: PathBuf,
    }

    impl Listening {
        /// Starts listening, unless another glave already does or the socket cannot be made;
        /// glave runs the same either way.
        pub async fn start(server: Glave) -> Option<Self> {
            let path = socket().ok()?;
            let listener = match UnixListener::bind(&path) {
                Ok(listener) => listener,
                Err(_) if Peer::at(&path).await.is_some() => return None,
                Err(_) => {
                    // A socket left by a glave that died: nobody answers on it.
                    let _ = tokio::fs::remove_file(&path).await;
                    UnixListener::bind(&path).ok()?
                }
            };
            let task = tokio::spawn(accept(listener, server));
            Some(Self { task, path })
        }

        /// Stops listening and removes the socket.
        pub async fn stop(self) {
            self.task.abort();
            let _ = self.task.await;
            let _ = tokio::fs::remove_file(&self.path).await;
        }
    }

    /// Serves each command that connects, a few at a time; ends with its task, which ends the
    /// connections it owns.
    async fn accept(listener: UnixListener, server: Glave) {
        let mut peers = JoinSet::new();
        loop {
            while peers.try_join_next().is_some() {}
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            if peers.len() >= MOST_PEERS {
                continue;
            }
            let server = server.clone();
            peers.spawn(async move {
                let (input, output) = stream.into_split();
                let _ = glave_session::rpc::serve_peer(server, BufReader::new(input), output).await;
            });
        }
    }
}

#[cfg(not(unix))]
mod other {
    use super::{ApiError, Glave, Reply, Request};

    /// There is no local socket on this system, so commands always work for themselves.
    pub struct Peer;

    impl Peer {
        pub async fn connect() -> Option<Self> {
            None
        }

        pub async fn call(&mut self, _request: &Request) -> Result<Reply, ApiError> {
            Err(ApiError {
                code: -32603,
                message: "no running glave".into(),
                data: None,
            })
        }
    }

    pub struct Listening;

    impl Listening {
        pub async fn start(_server: Glave) -> Option<Self> {
            None
        }

        pub async fn stop(self) {}
    }
}
