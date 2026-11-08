//! Previews of what a thread's commands in the background serve (`specs/session.md`,
//! "Previews"): each is the dev server's own address on this computer's loopback, an origin of
//! its own and never glave's, whose pages are opened at `localhost`. Reached by the family the
//! server listens in: IPv4's loopback where it listens there, else IPv6's.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use glave_ports::computer::CommandId;

use crate::protocol::Preview;

/// The previews of what `listening` names, each command's addresses as its computer found
/// them: one for each port, in their order.
pub(crate) fn of(listening: &[(CommandId, Vec<SocketAddr>)]) -> Vec<Preview> {
    let mut previews = Vec::new();
    for (command, at) in listening {
        let mut ports: Vec<u16> = at.iter().map(SocketAddr::port).collect();
        ports.dedup();
        for port in ports {
            let four = at.iter().any(|at| at.port() == port && at.is_ipv4());
            let loopback = if four {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            } else {
                IpAddr::V6(Ipv6Addr::LOCALHOST)
            };
            previews.push(Preview {
                command: *command,
                port,
                url: format!("http://{}/", SocketAddr::new(loopback, port)),
            });
        }
    }
    previews
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(n: u128) -> CommandId {
        CommandId::from_uuid(uuid::Uuid::from_u128(
            0x0190_0000_0000_7000_8000_0000_0000_0100 + n,
        ))
    }

    fn at(addresses: &[&str]) -> Vec<SocketAddr> {
        addresses
            .iter()
            .map(|address| address.parse().unwrap())
            .collect()
    }

    #[test]
    fn a_preview_is_the_server_s_own_address_in_the_family_it_listens_in() {
        let listening = [
            (command(1), at(&["127.0.0.1:5173", "[::1]:5173"])),
            (command(2), at(&["[::1]:3000", "[::]:4000"])),
            (
                command(3),
                at(&["0.0.0.0:8000", "[::]:8000", "127.0.0.1:9229"]),
            ),
        ];
        let shown: Vec<(CommandId, u16, String)> = of(&listening)
            .into_iter()
            .map(|preview| (preview.command, preview.port, preview.url))
            .collect();
        assert_eq!(
            shown,
            [
                (command(1), 5173, "http://127.0.0.1:5173/".to_owned()),
                (command(2), 3000, "http://[::1]:3000/".to_owned()),
                (command(2), 4000, "http://[::1]:4000/".to_owned()),
                (command(3), 8000, "http://127.0.0.1:8000/".to_owned()),
                (command(3), 9229, "http://127.0.0.1:9229/".to_owned()),
            ]
        );
        assert_eq!(of(&[]), []);
    }
}
