//! A person's computer serves only once its network policy holds (`specs/computers.md`, *The
//! network*). A cluster's network plugin takes a new Pod into its policy a moment after the Pod
//! starts, and some let it out meanwhile (kind's kindnet, measured: a computer's first command
//! reached the Kubernetes API, CoreDNS and another Pod); one that ignores policies never takes
//! it in at all. So before glave serves there, it knocks at its node's kubelet, inside every
//! cluster and answering anyone the policy lets through, until the knocks go unanswered.

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::{Instant, sleep, timeout};

/// How long one knock waits for an answer. The node's own kernel answers a connection in
/// microseconds to a few milliseconds, loaded or not; what is really slow is a lost attempt,
/// tried again after a second, which outlasts this as it would any wait short of that.
const KNOCK: Duration = Duration::from_millis(200);
/// How many knocks in a row must go unanswered before the policy counts as holding: one could
/// be a lost packet; several in a row, from a node that would answer, are not.
const SILENT_KNOCKS: u32 = 3;
/// How long a policy has to take hold: seconds where the plugin enforces it, never where it
/// doesn't; within the 30 seconds a bridge waits for glave to serve.
pub(super) const HOLD_WITHIN: Duration = Duration::from_secs(20);
/// Between a knock that was answered and the next.
const AGAIN: Duration = Duration::from_millis(100);

/// What one knock came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Knocked {
    /// Nothing came back within [`KNOCK`]: what a policy that drops what it refuses does (kind's
    /// kindnet, Calico and Cilium, so GKE's Dataplane V2, do).
    Silence,
    /// Something came back, so the knock got somewhere, and it never counts as the policy
    /// holding: a node that answers slowly or oddly keeps the computer from serving rather than
    /// letting it serve unfenced. How long it took, and whether it was a refusal, which a plugin
    /// that rejects rather than drops (kube-router, so k3s's) sends for every knock.
    Answer { took: Duration, refused: bool },
}

/// How the fence came to hold: after how many knocks, and the slowest answer before it did.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Held {
    pub(super) knocks: u32,
    pub(super) slowest: Option<Duration>,
}

/// Why the fence never held, for the computer's log, which reaches the server's log alone: the
/// person is told only that their computer couldn't start safely ([`glave_computer::UNFENCED`]).
#[derive(Debug)]
pub(super) struct Unfenced {
    inside: SocketAddr,
    within: Duration,
    /// Whether every answer was a refusal.
    refused: bool,
}

impl fmt::Display for Unfenced {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (inside, within) = (self.inside, glave_ports::spoken(self.within));
        if self.refused {
            write!(
                f,
                "every knock at {inside} inside the cluster was refused for {within}: the \
                 cluster's network plugin rejects what a policy refuses rather than dropping it, \
                 which can't be told from a node that answers, so this computer stops rather \
                 than serve unfenced, and its next wake tries again"
            )
        } else {
            write!(
                f,
                "this computer still reaches {inside} inside the cluster after {within}, so the \
                 cluster's network plugin isn't enforcing its network policy; it stops rather \
                 than serve unfenced, and its next wake tries again"
            )
        }
    }
}

impl std::error::Error for Unfenced {}

/// Returns once [`SILENT_KNOCKS`] knocks in a row at `inside` go unanswered; fails, saying why,
/// if they still don't after `within`.
pub(super) async fn held(inside: SocketAddr, within: Duration) -> Result<Held, Unfenced> {
    held_by(|| knock(inside), inside, within).await
}

/// One knock at `inside`: a connection tried, closed as soon as it is made.
async fn knock(inside: SocketAddr) -> Knocked {
    let started = Instant::now();
    match timeout(KNOCK, TcpStream::connect(inside)).await {
        Err(_) => Knocked::Silence,
        Ok(connected) => Knocked::Answer {
            took: started.elapsed(),
            refused: connected
                .err()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::ConnectionRefused),
        },
    }
}

/// [`held`], with each knock made by `knock`.
async fn held_by<K, F>(mut knock: K, inside: SocketAddr, within: Duration) -> Result<Held, Unfenced>
where
    K: FnMut() -> F,
    F: Future<Output = Knocked>,
{
    let started = Instant::now();
    let (mut knocks, mut silent) = (0, 0);
    let (mut slowest, mut refused_all) = (None::<Duration>, true);
    loop {
        knocks += 1;
        match knock().await {
            Knocked::Silence => silent += 1,
            Knocked::Answer { took, refused } => {
                silent = 0;
                slowest = Some(slowest.map_or(took, |slowest| slowest.max(took)));
                refused_all &= refused;
            }
        }
        if silent >= SILENT_KNOCKS {
            return Ok(Held { knocks, slowest });
        }
        if started.elapsed() >= within {
            return Err(Unfenced {
                inside,
                within,
                refused: refused_all && slowest.is_some(),
            });
        }
        if silent == 0 {
            sleep(AGAIN).await;
        }
    }
}

/// `text` as the address `--fence` names: `host:port`, the host an IP address, an IPv6 one
/// bare or in brackets, as Kubernetes puts a node's address in (`$(GLAVE_NODE):10250`).
pub(super) fn address(text: &str) -> Result<SocketAddr, String> {
    let wrong = || format!("--fence {text:?} is not an address:port");
    let (host, port) = text.rsplit_once(':').ok_or_else(wrong)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let ip = host.parse().map_err(|_| wrong())?;
    let port = port.parse().map_err(|_| wrong())?;
    Ok(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests;
