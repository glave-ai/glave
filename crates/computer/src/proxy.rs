//! The outbound proxy (`specs/sandbox.md`, "The outbound proxy"): every connection a sandboxed
//! command makes goes out through it, which lets it through, refuses it, or holds it for the
//! person, by the host's name and never decrypting. A name is decided before it is looked up,
//! the addresses it resolves to are checked by glave's one rule, and the proxy connects to an
//! address it checked.

mod connection;
mod head;
mod hold;
mod network;
pub(crate) mod pump;
mod rules;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

pub use hold::Held;
pub use network::{Internet, Network};
pub use rules::{Allowed, Host, Mode, Rules};

/// How long the proxy waits for each thing it waits for.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A request's head to arrive, and an answer or a head to be taken.
    pub head: Duration,
    /// A name to be looked up.
    pub resolve: Duration,
    /// Each address to answer a connection.
    pub connect: Duration,
    /// A connection with nothing moving either way, before it is closed.
    pub idle: Duration,
    /// The person to answer a hold.
    pub hold: Duration,
    /// Connections at once, not counting those held; the rest are answered "busy".
    pub connections: usize,
    /// Connections held for the person at once; the rest are answered "busy".
    pub holds: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            head: Duration::from_secs(10),
            resolve: Duration::from_secs(10),
            connect: Duration::from_secs(10),
            idle: Duration::from_mins(5),
            hold: Duration::from_mins(10),
            connections: 256,
            holds: 16,
        }
    }
}

/// What every connection shares.
struct Shared<N> {
    rules: Rules,
    network: N,
    /// What a client presents as the proxy URL's password, which only the commands it was
    /// made for are given.
    secret: String,
    holds: Option<mpsc::Sender<Held>>,
    limits: Limits,
    /// Places for connections not held.
    connections: Arc<Semaphore>,
    /// Places for connections held.
    holding: Arc<Semaphore>,
}

/// The outbound proxy: rules, where it reaches hosts, the credential its clients present, and
/// who answers its holds.
pub struct Proxy<N> {
    shared: Arc<Shared<N>>,
}

impl<N: Network> Proxy<N> {
    /// A proxy deciding by `rules`, reaching hosts through `network`, within `limits`, for
    /// clients that present `secret` (`Proxy-Authorization: Basic`, any user name), as a proxy
    /// URL carries it: `http://t:<secret>@127.0.0.1:<port>`. Holds go to `holds`, each
    /// answered there; with none, a host the rules would hold is refused.
    pub fn new(
        rules: Rules,
        network: N,
        secret: impl Into<String>,
        holds: Option<mpsc::Sender<Held>>,
        limits: Limits,
    ) -> Self {
        let shared = Shared {
            rules,
            network,
            secret: secret.into(),
            holds,
            limits,
            connections: Arc::new(Semaphore::new(limits.connections)),
            holding: Arc::new(Semaphore::new(limits.holds)),
        };
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Serves the connections `listener` takes, each as a task this owns, until dropped, which
    /// ends them all. They are at most the connections and holds its limits allow.
    pub async fn serve(self, listener: TcpListener) {
        self.serve_on(listener).await;
    }

    /// Serves the connections a Unix socket's `listener` takes, as [`Proxy::serve`] does: the
    /// one way out of a Linux sandbox's network namespace of its own.
    #[cfg(target_os = "linux")]
    pub(crate) async fn serve_unix(self, listener: tokio::net::UnixListener) {
        self.serve_on(listener).await;
    }

    #[expect(
        clippy::infinite_loop,
        reason = "the proxy serves until its owner drops it, which ends every connection"
    )]
    async fn serve_on<L: Listening>(self, listener: L) {
        let mut connections = JoinSet::new();
        loop {
            while connections.try_join_next().is_some() {}
            let client = match listener.take().await {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(%error, "the outbound proxy could not take a connection");
                    // Out of files, say: let the connections it has end before taking more.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let Ok(place) = Arc::clone(&self.shared.connections).try_acquire_owned() else {
                L::turn_away(&client);
                continue;
            };
            let shared = Arc::clone(&self.shared);
            connections.spawn(async move {
                if let Err(error) = connection::serve(client, &shared, place).await {
                    tracing::debug!(%error, "a command's connection through the proxy ended");
                }
            });
        }
    }
}

/// Where commands' connections to the proxy arrive: a TCP port, or a Unix socket.
trait Listening: Send {
    type Client: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    /// The next connection.
    fn take(&self) -> impl Future<Output = io::Result<Self::Client>> + Send;

    /// Says "busy" to a connection over the cap: as much as its socket takes at once, since a
    /// full one isn't waited for.
    fn turn_away(client: &Self::Client);
}

impl Listening for TcpListener {
    type Client = TcpStream;

    async fn take(&self) -> io::Result<TcpStream> {
        self.accept().await.map(|(client, _)| client)
    }

    fn turn_away(client: &TcpStream) {
        let _ = client.try_write(BUSY);
    }
}

#[cfg(target_os = "linux")]
impl Listening for tokio::net::UnixListener {
    type Client = tokio::net::UnixStream;

    async fn take(&self) -> io::Result<Self::Client> {
        self.accept().await.map(|(client, _)| client)
    }

    fn turn_away(client: &Self::Client) {
        let _ = client.try_write(BUSY);
    }
}

/// The answer to a connection over the cap.
const BUSY: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
