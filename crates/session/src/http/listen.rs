//! Where the session API listens: plain HTTP (on this computer, both its loopback addresses at
//! one port, so `localhost` reaches this server alone), and beside it HTTPS of the server's
//! own, each listener with the names it answers to (`hosts`). A connection to HTTPS is served
//! once its handshake completes, within the time a client has to send its headers; one whose
//! handshake fails, as every one does before a certificate is in hand, is closed.

use std::future::poll_fn;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::task::Poll;

use glave_ports::host::PublicOrigin;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use super::HEADER_TIMEOUT;
use super::hosts::ListenerNames;

/// Where a server listens over HTTP: in the clear (one address, or both of this computer's
/// loopback addresses at one port), and for HTTPS beside it, with what its handshakes are
/// answered with.
pub struct Listeners {
    pub plain: Vec<TcpListener>,
    pub secure: Option<(TcpListener, Arc<ServerConfig>)>,
}

impl From<TcpListener> for Listeners {
    fn from(plain: TcpListener) -> Self {
        Self {
            plain: vec![plain],
            secure: None,
        }
    }
}

/// One listener, with the names it answers to and, for HTTPS, its handshakes.
pub(crate) struct Listening {
    listener: TcpListener,
    pub(crate) names: Arc<Option<ListenerNames>>,
    tls: Option<TlsAcceptor>,
}

/// A connection's stream, in the clear or over TLS.
pub(crate) trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

impl Listeners {
    /// The port browsers on this computer reach the server at as `localhost`: one it listens
    /// on in the clear at both loopback addresses, so no one else answers that name there.
    pub(crate) fn localhost(&self) -> Option<u16> {
        let bound: Vec<SocketAddr> = self
            .plain
            .iter()
            .filter_map(|listener| listener.local_addr().ok())
            .collect();
        let at = |ip: IpAddr| {
            bound
                .iter()
                .filter(move |address| address.ip() == ip)
                .map(SocketAddr::port)
        };
        at(IpAddr::V4(Ipv4Addr::LOCALHOST))
            .find(|port| at(IpAddr::V6(Ipv6Addr::LOCALHOST)).any(|other| other == *port))
    }

    /// Each listener with the names it answers to, the server reached from elsewhere at
    /// `origin` if anywhere: those in the clear first.
    pub(crate) fn listening(
        self,
        origin: Option<&PublicOrigin>,
    ) -> std::io::Result<Vec<Listening>> {
        let names = |listener: &TcpListener| {
            Ok::<_, std::io::Error>(Arc::new(ListenerNames::of(listener.local_addr()?, origin)))
        };
        let mut all = Vec::with_capacity(self.plain.len() + 1);
        for listener in self.plain {
            all.push(Listening {
                names: names(&listener)?,
                listener,
                tls: None,
            });
        }
        if let Some((listener, config)) = self.secure {
            all.push(Listening {
                names: names(&listener)?,
                listener,
                tls: Some(TlsAcceptor::from(config)),
            });
        }
        Ok(all)
    }
}

/// The next connection to any of `all`, with the listener it came to.
pub(crate) async fn accept(
    all: &[Listening],
) -> (std::io::Result<(TcpStream, SocketAddr)>, &Listening) {
    poll_fn(|context| {
        for listening in all {
            if let Poll::Ready(accepted) = listening.listener.poll_accept(context) {
                return Poll::Ready((accepted, listening));
            }
        }
        Poll::Pending
    })
    .await
}

impl Listening {
    /// What a connection to this listener is opened with: its handshakes, for HTTPS.
    pub(crate) fn handshakes(&self) -> Option<TlsAcceptor> {
        self.tls.clone()
    }
}

/// `stream`, ready to read requests from: itself in the clear, or once its TLS handshake with
/// `tls` completes; none when that fails or takes too long.
pub(crate) async fn connected(
    tls: Option<TlsAcceptor>,
    stream: TcpStream,
) -> Option<Box<dyn Stream>> {
    let Some(tls) = tls else {
        return Some(Box::new(stream));
    };
    match tokio::time::timeout(HEADER_TIMEOUT, tls.accept(stream)).await {
        Ok(Ok(secured)) => Some(Box::new(secured)),
        _ => None,
    }
}
