//! The outbound proxy a prepared sandbox's commands go out through (`specs/sandbox.md`, "The
//! outbound proxy"): one per prepared sandbox, so one per thread, on a Unix socket in the
//! sandbox's own folder, with a secret of its own that only its commands are given.

use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_ports::computer::ComputerError;
use glave_ports::{BoxFuture, Entropy as _, MAX_SOCKET_PATH, OsEntropy};
use tokio::net::UnixListener;
use tokio::task::JoinSet;

use crate::proxy::{Internet, Limits, Mode, Network, Proxy, Rules};

/// What serves a proxy on a socket's listener, for clients presenting the secret.
type Serve = dyn Fn(UnixListener, String) -> BoxFuture<'static, ()> + Send + Sync;

/// How a sandbox's proxy is made: its rules and where it reaches hosts. By default any public
/// host, through this computer's resolver and sockets, until settings choose a mode.
#[derive(Clone)]
pub(crate) struct Outbound(Arc<Serve>);

impl Outbound {
    /// Proxies deciding by `rules`, reaching hosts through `network`.
    pub(crate) fn over<N: Network + Clone>(rules: Rules, network: N) -> Self {
        Self(Arc::new(move |listener, secret| {
            let network = network.clone();
            let proxy = Proxy::new(rules.clone(), network, secret, None, Limits::default());
            Box::pin(proxy.serve_unix(listener))
        }))
    }
}

impl Default for Outbound {
    fn default() -> Self {
        Self::over(Rules::new(Mode::Open, []), Internet)
    }
}

impl fmt::Debug for Outbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Outbound")
    }
}

/// The socket's name in the sandbox's folder.
const SOCKET: &str = "proxy.sock";

/// Random bytes in a proxy's secret: two draws of glave's entropy, 160 bits.
const DRAWS: usize = 2;

/// A sandbox's proxy while it serves: the socket its commands' forwarders reach, and the
/// secret they present. Dropped, it stops, ending every connection through it.
pub(super) struct Serving {
    socket: PathBuf,
    secret: String,
    _proxy: JoinSet<()>,
}

impl Serving {
    /// Starts `outbound`'s proxy on a socket in `folder`, private to this user, with a new
    /// secret.
    pub(super) fn start(outbound: &Outbound, folder: &Path) -> Result<Self, ComputerError> {
        let socket = folder.join(SOCKET);
        if socket.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(ComputerError::Failed(format!(
                "the outbound proxy's socket would be too long a path: {}",
                socket.display()
            )));
        }
        let listener = UnixListener::bind(&socket).map_err(|error| {
            ComputerError::Failed(format!(
                "could not start the outbound proxy at {}: {error}",
                socket.display()
            ))
        })?;
        let secret = secret()?;
        let mut proxy = JoinSet::new();
        proxy.spawn((outbound.0)(listener, secret.clone()));
        Ok(Self {
            socket,
            secret,
            _proxy: proxy,
        })
    }

    /// Where the forwarder in a command's namespace reaches the proxy.
    pub(super) fn socket(&self) -> &Path {
        &self.socket
    }

    /// What the command presents to the proxy.
    pub(super) fn secret(&self) -> &str {
        &self.secret
    }
}

impl fmt::Debug for Serving {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Serving")
            .field("socket", &self.socket)
            .field("secret", &"<hidden>")
            .finish_non_exhaustive()
    }
}

/// A new secret from glave's entropy, in hex, or why there is none: a proxy is never left with a
/// weak one.
fn secret() -> Result<String, ComputerError> {
    let mut secret = String::new();
    for _ in 0..DRAWS {
        let bytes = OsEntropy.bytes().map_err(|error| {
            ComputerError::Failed(format!("could not make the proxy's secret: {error}"))
        })?;
        for byte in bytes {
            let _ = write!(secret, "{byte:02x}");
        }
    }
    Ok(secret)
}
