//! The outbound proxy a prepared sandbox's commands go out through (`specs/sandbox.md`, "The
//! outbound proxy"): one per prepared sandbox, so one per thread, with a secret of its own that
//! only its commands are given. On Linux it listens on a Unix socket in the sandbox's own
//! folder, which a forwarder in each command's network namespace reaches; on macOS on a port of
//! this computer's loopback, which the command reaches itself.

use std::fmt::{self, Write as _};
use std::sync::Arc;

use glave_ports::computer::ComputerError;
use glave_ports::{BoxFuture, Entropy as _, OsEntropy};
use tokio::task::JoinSet;

use crate::proxy::{Internet, Limits, Mode, Network, Proxy, Rules};

/// Where a proxy takes its commands' connections: a Unix socket on Linux, loopback on macOS.
#[cfg(target_os = "linux")]
pub(super) type Listener = tokio::net::UnixListener;
#[cfg(target_os = "macos")]
pub(super) type Listener = tokio::net::TcpListener;

/// What serves a proxy on a listener, for clients presenting the secret.
type Serve = dyn Fn(Listener, String) -> BoxFuture<'static, ()> + Send + Sync;

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
            serve(proxy, listener)
        }))
    }
}

#[cfg(target_os = "linux")]
fn serve<N: Network>(proxy: Proxy<N>, listener: Listener) -> BoxFuture<'static, ()> {
    Box::pin(proxy.serve_unix(listener))
}

#[cfg(target_os = "macos")]
fn serve<N: Network>(proxy: Proxy<N>, listener: Listener) -> BoxFuture<'static, ()> {
    Box::pin(proxy.serve(listener))
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

/// Random bytes in a proxy's secret: two draws of glave's entropy, 160 bits.
const DRAWS: usize = 2;

/// The variables that name the proxy, as programs read them.
pub(super) const PROXY: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// The variables that name what is reached without the proxy: loopback, which the proxy
/// refuses.
pub(super) const NO_PROXY: [&str; 2] = ["NO_PROXY", "no_proxy"];

/// Loopback, by every name it goes by.
const LOOPBACK: &str = "localhost,127.0.0.1,::1";

/// What a command kept to the proxy at `port` on loopback is given, with `secret` in its URL.
pub(super) fn variables(secret: &str, port: u16) -> Vec<(&'static str, String)> {
    let url = format!("http://t:{secret}@127.0.0.1:{port}");
    PROXY
        .map(|name| (name, url.clone()))
        .into_iter()
        .chain(NO_PROXY.map(|name| (name, LOOPBACK.to_owned())))
        .collect()
}

/// A sandbox's proxy while it serves, and the secret its commands present. Dropped, it stops,
/// ending every connection through it.
pub(super) struct Serving {
    secret: String,
    _proxy: JoinSet<()>,
}

impl Serving {
    /// Starts `outbound`'s proxy on `listener`, with a new secret.
    pub(super) fn start(outbound: &Outbound, listener: Listener) -> Result<Self, ComputerError> {
        let secret = secret()?;
        let mut proxy = JoinSet::new();
        proxy.spawn((outbound.0)(listener, secret.clone()));
        Ok(Self {
            secret,
            _proxy: proxy,
        })
    }

    /// What the command presents to the proxy.
    pub(super) fn secret(&self) -> &str {
        &self.secret
    }
}

impl fmt::Debug for Serving {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Serving")
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
