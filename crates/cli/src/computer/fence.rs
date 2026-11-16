//! A person's computer serves only once its network policy holds (`specs/computers.md`, *The
//! network*). A cluster's network plugin takes a new Pod's policy in a moment after the Pod
//! starts, and some let it out meanwhile (kind's kindnet, measured: a computer's first command
//! reached the Kubernetes API, CoreDNS and another Pod); one that ignores policies never takes
//! it in at all. So before glave serves there, it tries its node's kubelet, inside every
//! cluster and answering anyone the policy lets through, until it is no longer reached.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::bail;
use tokio::net::TcpStream;
use tokio::time::{Instant, sleep, timeout};

/// How long one try waits for an answer: the kubelet, inside the cluster, answers within
/// milliseconds, so silence this long is the policy dropping the connection.
const TRY: Duration = Duration::from_millis(500);
/// How long a policy has to take hold: seconds where the plugin enforces it, never where it
/// doesn't; within the 30 seconds a bridge waits for glave to serve.
pub(super) const HOLD_WITHIN: Duration = Duration::from_secs(20);
/// Between one try that got through and the next.
const AGAIN: Duration = Duration::from_millis(200);

/// Returns once a connection to `inside` is no longer made, or fails, saying so, if one still
/// is after `within`.
pub(super) async fn held(inside: SocketAddr, within: Duration) -> anyhow::Result<()> {
    let started = Instant::now();
    loop {
        let reached = matches!(timeout(TRY, TcpStream::connect(inside)).await, Ok(Ok(_)));
        if !reached {
            return Ok(());
        }
        if started.elapsed() >= within {
            bail!(
                "this computer still reaches {inside} inside the cluster after {}, so the \
                 cluster's network plugin isn't enforcing its network policy; it will not \
                 serve until it does",
                glave_ports::spoken(within)
            );
        }
        sleep(AGAIN).await;
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
mod tests {
    use std::net::Ipv4Addr;

    use glave_ports::block_on;
    use tokio::net::TcpListener;

    use super::{address, held};

    #[test]
    fn an_address_no_longer_reached_is_held_at_once() {
        block_on!(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let closed = listener.local_addr().unwrap();
            drop(listener);
            held(closed, super::HOLD_WITHIN).await.unwrap();
        });
    }

    #[test]
    fn an_address_still_reached_is_never_held_and_says_where() {
        block_on!(async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let open = listener.local_addr().unwrap();
            let said = held(open, std::time::Duration::from_millis(300))
                .await
                .unwrap_err()
                .to_string();
            assert!(said.contains(&open.to_string()), "{said}");
            assert!(said.contains("network policy"), "{said}");
        });
    }

    #[test]
    fn a_node_s_address_is_read_with_its_port() {
        assert_eq!(
            address("10.0.0.7:10250").unwrap().to_string(),
            "10.0.0.7:10250"
        );
        assert_eq!(
            address("fd00::7:10250").unwrap().to_string(),
            "[fd00::7]:10250"
        );
        assert_eq!(
            address("[fd00::7]:10250").unwrap().to_string(),
            "[fd00::7]:10250"
        );
        for wrong in ["", "10.0.0.7", "node:10250", "10.0.0.7:port", ":10250"] {
            assert!(address(wrong).is_err(), "{wrong}");
        }
    }
}
