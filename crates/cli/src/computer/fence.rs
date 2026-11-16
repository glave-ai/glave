//! A person's computer serves only once its network policy holds (`specs/computers.md`, *The
//! network*). A cluster's network plugin takes a new Pod into its policy a moment after the Pod
//! starts, and some let it out meanwhile (kind's kindnet, measured: a computer's first command
//! reached the Kubernetes API, CoreDNS and another Pod); one that ignores policies never takes
//! it in at all. So before glave serves there, it knocks at its node's kubelet, inside every
//! cluster and answering anyone the policy lets through, until the knocks go unanswered.

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::bail;
use tokio::net::TcpStream;
use tokio::time::{Instant, sleep, timeout};

/// How long one knock waits for an answer. The node's own kernel answers a connection within
/// milliseconds, a loaded node's and one behind a quarter of a CPU's limit well within this.
const KNOCK: Duration = Duration::from_millis(500);
/// How many knocks in a row must go unanswered before the policy counts as holding: one could
/// be a lost packet or an answer slower than [`KNOCK`]; several in a row, from a node that
/// would answer, are neither.
const SILENT_KNOCKS: u32 = 3;
/// How long a policy has to take hold: seconds where the plugin enforces it, never where it
/// doesn't; within the 30 seconds a bridge waits for glave to serve.
pub(super) const HOLD_WITHIN: Duration = Duration::from_secs(20);
/// Between a knock that was answered and the next.
const AGAIN: Duration = Duration::from_millis(200);

/// What one knock came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Knocked {
    /// Nothing came back within [`KNOCK`]: a policy drops what it refuses, as every plugin
    /// that enforces one does.
    Silence,
    /// Something came back: a connection, a refusal or an unreachable. Each means the knock
    /// got somewhere, so none counts as the policy holding, and a node that answers oddly keeps
    /// the computer from serving rather than letting it serve unfenced.
    Answer,
}

/// Returns once [`SILENT_KNOCKS`] knocks in a row at `inside` go unanswered, saying how many it
/// took; fails, saying so, if they still don't after `within`.
pub(super) async fn held(inside: SocketAddr, within: Duration) -> anyhow::Result<u32> {
    held_by(|| knock(inside), inside, within).await
}

/// One knock at `inside`: a connection tried, closed as soon as it is made.
async fn knock(inside: SocketAddr) -> Knocked {
    match timeout(KNOCK, TcpStream::connect(inside)).await {
        Err(_) => Knocked::Silence,
        Ok(_) => Knocked::Answer,
    }
}

/// [`held`], with each knock made by `knock`.
async fn held_by<K, F>(mut knock: K, inside: SocketAddr, within: Duration) -> anyhow::Result<u32>
where
    K: FnMut() -> F,
    F: Future<Output = Knocked>,
{
    let started = Instant::now();
    let (mut knocks, mut silent) = (0, 0);
    loop {
        knocks += 1;
        match knock().await {
            Knocked::Silence => silent += 1,
            Knocked::Answer => silent = 0,
        }
        if silent >= SILENT_KNOCKS {
            return Ok(knocks);
        }
        if started.elapsed() >= within {
            bail!(
                "this computer still reaches {inside} inside the cluster after {}, so the \
                 cluster's network plugin isn't enforcing its network policy; it stops rather \
                 than serve unfenced, and its next wake tries again",
                glave_ports::spoken(within)
            );
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
