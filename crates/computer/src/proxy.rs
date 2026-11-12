//! The outbound proxy (`specs/sandbox.md`, "The outbound proxy"): every connection a sandboxed
//! command makes goes out through it, which lets it through, refuses it, or holds it for the
//! person, by the host's name and never decrypting. A name is decided before it is looked up,
//! the addresses it resolves to are checked by glave's one rule, and the proxy connects to an
//! address it checked.

mod connection;
mod head;
mod hold;
mod network;
mod pump;
mod rules;

use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
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
    #[expect(
        clippy::infinite_loop,
        reason = "the proxy serves until its owner drops it, which ends every connection"
    )]
    pub async fn serve(self, listener: TcpListener) {
        let mut connections = JoinSet::new();
        loop {
            while connections.try_join_next().is_some() {}
            let client = match listener.accept().await {
                Ok((client, _)) => client,
                Err(error) => {
                    tracing::warn!(%error, "the outbound proxy could not take a connection");
                    // Out of files, say: let the connections it has end before taking more.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let Ok(place) = Arc::clone(&self.shared.connections).try_acquire_owned() else {
                // As much as the socket takes at once: a full one isn't waited for.
                let _ = client.try_write(BUSY);
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

/// The answer to a connection over the cap.
const BUSY: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
