//! The outbound proxy (`specs/sandbox.md`, "The outbound proxy"): every connection a sandboxed
//! command makes goes out through it, which lets it through, refuses it, or holds it for the
//! person, by the host's name and never decrypting. A name is decided before it is looked up,
//! the addresses it resolves to are checked by glave's one rule, and the proxy connects to an
//! address it checked.

mod connection;
mod head;
mod network;
mod pump;
mod rules;

use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

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
    /// Connections at once; the rest are answered "busy".
    pub connections: usize,
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
        }
    }
}

/// A connection held for the person: the host it names, and the answer it waits for.
#[derive(Debug)]
pub struct Held {
    host: Host,
    answer: oneshot::Sender<bool>,
}

impl Held {
    fn new(host: Host, answer: oneshot::Sender<bool>) -> Self {
        Self { host, answer }
    }

    /// The host the connection names.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// Lets the connection through (`yes`) or refuses it. A connection that has given up
    /// waiting is not affected.
    pub fn answer(self, yes: bool) {
        // The connection may have stopped waiting; then no one needs the answer.
        let _ = self.answer.send(yes);
    }
}

/// What every connection shares.
struct Shared<N> {
    rules: Rules,
    network: N,
    holds: Option<mpsc::Sender<Held>>,
    limits: Limits,
}

/// The outbound proxy: rules, where it reaches hosts, and who answers its holds.
pub struct Proxy<N> {
    shared: Arc<Shared<N>>,
}

impl<N: Network> Proxy<N> {
    /// A proxy deciding by `rules`, reaching hosts through `network`, within `limits`. Holds go
    /// to `holds`, each answered there; with none, a host the rules would hold is refused.
    pub fn new(
        rules: Rules,
        network: N,
        holds: Option<mpsc::Sender<Held>>,
        limits: Limits,
    ) -> Self {
        let shared = Shared {
            rules,
            network,
            holds,
            limits,
        };
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Serves the connections `listener` takes, each as a task this owns, until dropped, which
    /// ends them all.
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
            if connections.len() >= self.shared.limits.connections {
                connections.spawn(busy(client, self.shared.limits));
                continue;
            }
            let shared = Arc::clone(&self.shared);
            connections.spawn(async move {
                if let Err(error) = connection::serve(client, &shared).await {
                    tracing::debug!(%error, "a command's connection through the proxy ended");
                }
            });
        }
    }
}

/// Answers a connection over the cap "busy" and closes it.
async fn busy(mut client: tokio::net::TcpStream, limits: Limits) {
    use tokio::io::AsyncWriteExt;
    let busy =
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let _ = tokio::time::timeout(limits.head, client.write_all(busy)).await;
}
