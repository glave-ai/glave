//! Where the session API listens: plain HTTP, and beside it HTTPS of the server's own, each
//! listener with the names it answers to (`hosts`). A connection to HTTPS is served once its
//! handshake completes, within the time a client has to send its headers; one whose handshake
//! fails, as every one does before a certificate is in hand, is closed.

use std::future::pending;
use std::net::SocketAddr;
use std::sync::Arc;

use glave_ports::host::PublicOrigin;
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use super::HEADER_TIMEOUT;
use super::hosts::ListenerNames;

/// Where a server listens over HTTP: one listener in the clear, and one for HTTPS beside it,
/// with what its handshakes are answered with.
pub struct Listeners {
    pub plain: TcpListener,
    pub secure: Option<(TcpListener, Arc<ServerConfig>)>,
}

impl From<TcpListener> for Listeners {
    fn from(plain: TcpListener) -> Self {
        Self {
            plain,
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
    /// Each listener with the names it answers to, the server reached from elsewhere at
    /// `origin` if anywhere.
    pub(crate) fn listening(
        self,
        origin: Option<&PublicOrigin>,
    ) -> std::io::Result<(Listening, Option<Listening>)> {
        let names = |listener: &TcpListener| {
            Ok::<_, std::io::Error>(Arc::new(ListenerNames::of(listener.local_addr()?, origin)))
        };
        let plain = Listening {
            names: names(&self.plain)?,
            listener: self.plain,
            tls: None,
        };
        let secure = match self.secure {
            Some((listener, config)) => Some(Listening {
                names: names(&listener)?,
                listener,
                tls: Some(TlsAcceptor::from(config)),
            }),
            None => None,
        };
        Ok((plain, secure))
    }
}

impl Listening {
    /// The next connection to this listener, or to `other`'s, with the listener it came to.
    pub(crate) async fn accept<'a>(
        &'a self,
        other: Option<&'a Self>,
    ) -> (std::io::Result<(TcpStream, SocketAddr)>, &'a Self) {
        let others = async {
            match other {
                Some(other) => (other.listener.accept().await, other),
                None => pending().await,
            }
        };
        tokio::select! {
            accepted = self.listener.accept() => (accepted, self),
            accepted = others => accepted,
        }
    }

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
