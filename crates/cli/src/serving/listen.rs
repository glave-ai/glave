//! Where the person's server listens over HTTP: bound as it starts, at 127.0.0.1 with ::1
//! beside it, so `localhost` reaches this server alone.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use anyhow::Context as _;
use tokio::net::TcpListener;

/// What the person's server listens on over HTTP, bound as it starts: its listener, and the one
/// at ::1 beside it at 127.0.0.1, or why that could not be had ([`localhost_beside`]).
#[derive(Debug)]
pub(crate) struct Listening {
    pub(super) listener: TcpListener,
    pub(super) beside: std::io::Result<Option<TcpListener>>,
}

impl Listening {
    /// Where its listener listens.
    pub(crate) fn address(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

/// Where the person's server listens when nothing else is chosen, bound in the step that finds
/// it free, so no other program takes it between: 127.0.0.1 at the usual port when it is free
/// there and at ::1, which `localhost` also stands for, else at a port that is, else at any
/// free one.
pub(crate) async fn listen_here() -> anyhow::Result<Listening> {
    let ports = std::iter::once(crate::server::PORT).chain(std::iter::repeat_n(0, 8));
    for port in ports {
        if let Some(listening) = free_here(port).await {
            return Ok(listening);
        }
    }
    listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await
}

/// Listening at `port` (any free one for 0), at 127.0.0.1 and beside it at ::1; none when
/// either is taken. Any other trouble at ::1 is kept for [`localhost_beside`] to weigh.
async fn free_here(port: u16) -> Option<Listening> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await.ok()?;
    match beside(listener.local_addr().ok()?).await {
        Err(error) if error.kind() == ErrorKind::AddrInUse => None,
        beside => Some(Listening { listener, beside }),
    }
}

/// Listens on `address`, which the person chose, and on ::1 beside it at 127.0.0.1
/// ([`beside`]), at once. Where it is the usual port and taken: any free one. An origin naming
/// that port then refuses to start (`served_at`); one of `localhost` without a port follows
/// it.
pub(crate) async fn listen(address: SocketAddr) -> anyhow::Result<Listening> {
    let listener = bind(address).await?;
    let beside = beside(listener.local_addr()?).await;
    Ok(Listening { listener, beside })
}

/// A listener on `address`, or on any free port beside it where the usual one is taken.
async fn bind(address: SocketAddr) -> anyhow::Result<TcpListener> {
    match TcpListener::bind(address).await {
        Ok(listener) => Ok(listener),
        Err(error)
            if error.kind() == ErrorKind::AddrInUse
                && address.ip().is_loopback()
                && address.port() == crate::server::PORT =>
        {
            let any = SocketAddr::new(address.ip(), 0);
            TcpListener::bind(any)
                .await
                .with_context(|| format!("could not listen on {any}"))
        }
        Err(error) => Err(error).with_context(|| format!("could not listen on {address}")),
    }
}

/// The listener at ::1 that `beside` holds, bound beside one at 127.0.0.1 at `port`, so
/// `localhost`, which stands for both, reaches this server alone; none for any other address,
/// or when it couldn't be had. Where `localhost` is the origin people share this computer by
/// (`shared`), glave must hold it, so no other program answers a browser there, unless this
/// computer has no ::1 at all.
pub(super) fn localhost_beside(
    beside: std::io::Result<Option<TcpListener>>,
    port: u16,
    shared: bool,
) -> anyhow::Result<Option<TcpListener>> {
    match beside {
        Ok(beside) => Ok(beside),
        Err(error) if shared && error.kind() != ErrorKind::AddrNotAvailable => Err(error)
            .with_context(|| {
                format!(
                    "could not listen on [::1]:{port}, where a browser may reach \
                     localhost:{port} too, so another program would answer people there: stop \
                     what listens there, or serve at another port and set the origin to it"
                )
            }),
        Err(_) => Ok(None),
    }
}

/// A listener at ::1 beside one `bound` at 127.0.0.1, at the same port; none for any other.
async fn beside(bound: SocketAddr) -> std::io::Result<Option<TcpListener>> {
    if bound.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) {
        return Ok(None);
    }
    let v6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), bound.port());
    TcpListener::bind(v6).await.map(Some)
}

/// The address a browser on this computer reaches `bound` at: itself, or loopback for a
/// listener on every address.
pub(super) fn here(bound: SocketAddr) -> SocketAddr {
    let ip = match bound.ip() {
        IpAddr::V4(v4) if v4.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(v6) if v6.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bound.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_is_free_here_only_at_both_loopback_addresses_and_stays_held() {
        glave_ports::block_on!(async {
            let listening = free_here(0).await.unwrap();
            let port = listening.address().unwrap().port();
            assert_ne!(port, 0);
            // Found free and bound in one step: no other program takes it meanwhile.
            let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await;
            assert_eq!(taken.unwrap_err().kind(), ErrorKind::AddrInUse);
            let v6 = listening.beside.unwrap().unwrap();
            assert_eq!(
                v6.local_addr().unwrap().port(),
                port,
                "the same port beside it"
            );

            let held = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
            let port = held.local_addr().unwrap().port();
            assert!(free_here(port).await.is_none(), "taken at [::1]");
        });
    }

    #[test]
    fn a_loopback_listener_holds_its_port_at_both_addresses_so_localhost_is_this_server() {
        glave_ports::block_on!(async {
            let v4 = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let bound = v4.local_addr().unwrap();
            let port = bound.port();
            let v6 = localhost_beside(beside(bound).await, port, false)
                .unwrap()
                .unwrap();
            let expected = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), bound.port());
            assert_eq!(v6.local_addr().unwrap(), expected);
            let taken = localhost_beside(beside(bound).await, port, false).unwrap();
            assert!(taken.is_none(), "taken there: no localhost");
            let refused = localhost_beside(beside(bound).await, port, true).unwrap_err();
            assert!(
                format!("{refused:#}").contains("another program"),
                "shared, it must be glave's: {refused:#}"
            );
            let any = "0.0.0.0:4180".parse().unwrap();
            assert!(
                localhost_beside(beside(any).await, 4180, true)
                    .unwrap()
                    .is_none()
            );
        });
    }

    #[test]
    fn a_link_points_at_this_computer() {
        let at = |text: &str| here(text.parse().unwrap()).to_string();
        assert_eq!(at("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(at("[::]:8080"), "[::1]:8080");
        assert_eq!(at("127.0.0.1:9"), "127.0.0.1:9");
    }
}
