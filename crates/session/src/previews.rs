//! Where a command in the background listens, as a client is told it, and the previews of what
//! it serves (`specs/session.md`, "Previews"): each the server's own address on this computer's
//! loopback, an origin of its own and never glave's, whose pages are opened at `localhost`.
//! A server listening on IPv4 or on every address of IPv6 (`::`, which takes IPv4 too) is
//! reached at `127.0.0.1`, which a frame of the app shows; one on IPv6's loopback alone at
//! `[::1]`, which a policy's sources can't name, so it opens in a new tab only.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use glave_harness::Port;

/// Why a preview opens in a new tab alone, as a client is told it.
const UNFRAMED: &str =
    "It answers only at ::1, which glave can't show inside its page: open it in a new tab.";

/// The ports listened on at `at`, lowest first, as its computer found them; each with its
/// preview where the server previews (`previewed`).
pub(crate) fn ports(at: &[SocketAddr], previewed: bool) -> Vec<Port> {
    let mut ports: Vec<u16> = at.iter().map(SocketAddr::port).collect();
    ports.sort_unstable();
    ports.dedup();
    ports
        .into_iter()
        .map(|port| {
            // IPv4 at all, or every address of IPv6, which takes IPv4 too: 127.0.0.1 reaches it.
            let four = at
                .iter()
                .filter(|at| at.port() == port)
                .any(|at| at.is_ipv4() || at.ip().is_unspecified());
            let loopback = if four {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            } else {
                IpAddr::V6(Ipv6Addr::LOCALHOST)
            };
            let url = format!("http://{}/", SocketAddr::new(loopback, port));
            Port {
                port,
                url: previewed.then_some(url),
                framed: previewed && four,
                unframed: (previewed && !four).then(|| UNFRAMED.to_owned()),
            }
        })
        .collect()
}

/// Where a command listens, in a sentence, from `ports`: "Listening on 3000, 4200 and 4201.";
/// none where it listens nowhere.
pub(crate) fn listens(ports: &[Port]) -> Option<String> {
    let named: Vec<String> = ports.iter().map(|port| port.port.to_string()).collect();
    let (last, rest) = named.split_last()?;
    Some(if rest.is_empty() {
        format!("Listening on {last}.")
    } else {
        format!("Listening on {} and {last}.", rest.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(addresses: &[&str]) -> Vec<SocketAddr> {
        addresses
            .iter()
            .map(|address| address.parse().unwrap())
            .collect()
    }

    fn shown(ports: &[Port]) -> Vec<(u16, Option<&str>, bool)> {
        ports
            .iter()
            .map(|port| (port.port, port.url.as_deref(), port.framed))
            .collect()
    }

    #[test]
    fn a_preview_is_the_server_s_own_address_at_127_0_0_1_wherever_that_reaches_it() {
        // IPv4's loopback or every address, and every address of IPv6: a frame shows it.
        for bound in [
            &["127.0.0.1:5173"][..],
            &["0.0.0.0:5173"],
            &["[::]:5173"],
            &["127.0.0.1:5173", "[::1]:5173"],
        ] {
            let ports = ports(&at(bound), true);
            assert_eq!(
                shown(&ports),
                [(5173, Some("http://127.0.0.1:5173/"), true)],
                "{bound:?}"
            );
            assert_eq!(ports[0].unframed, None);
        }
        // IPv6's loopback alone, as Vite binds `localhost` on many Macs: a new tab alone.
        let six = ports(&at(&["[::1]:5173"]), true);
        assert_eq!(shown(&six), [(5173, Some("http://[::1]:5173/"), false)]);
        assert!(
            six[0]
                .unframed
                .as_deref()
                .is_some_and(|why| why.contains("new tab"))
        );
    }

    #[test]
    fn each_port_is_said_once_lowest_first_and_previewed_only_where_the_server_previews() {
        let both = at(&["[::]:4000", "127.0.0.1:3000", "0.0.0.0:4000"]);
        let previewed = ports(&both, true);
        assert_eq!(
            shown(&previewed),
            [
                (3000, Some("http://127.0.0.1:3000/"), true),
                (4000, Some("http://127.0.0.1:4000/"), true),
            ]
        );
        let not = ports(&both, false);
        assert_eq!(shown(&not), [(3000, None, false), (4000, None, false)]);
        assert!(not.iter().all(|port| port.unframed.is_none()));
    }

    #[test]
    fn where_a_command_listens_reads_as_a_sentence() {
        let said = |bound: &[&str]| listens(&ports(&at(bound), false));
        assert_eq!(said(&[]), None);
        assert_eq!(
            said(&["127.0.0.1:4200"]).as_deref(),
            Some("Listening on 4200.")
        );
        assert_eq!(
            said(&["127.0.0.1:4201", "127.0.0.1:4200"]).as_deref(),
            Some("Listening on 4200 and 4201.")
        );
        assert_eq!(
            said(&["127.0.0.1:3000", "127.0.0.1:4200", "127.0.0.1:4201"]).as_deref(),
            Some("Listening on 3000, 4200 and 4201.")
        );
    }
}
