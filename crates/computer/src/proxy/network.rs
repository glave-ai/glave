//! Where the proxy looks names up and connects: the internet, through this computer's resolver
//! and sockets, or a stand-in for it in tests.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::net::TcpStream;

/// How the proxy reaches hosts: what a name resolves to, and a connection to an address.
pub trait Network: Send + Sync + 'static {
    /// The addresses `name` resolves to.
    fn resolve(&self, name: &str) -> impl Future<Output = io::Result<Vec<IpAddr>>> + Send;

    /// A connection to `to`, an address the proxy checked.
    fn connect(&self, to: SocketAddr) -> impl Future<Output = io::Result<TcpStream>> + Send;
}

/// The internet, through this computer's resolver and sockets.
#[derive(Debug, Clone, Copy, Default)]
pub struct Internet;

impl Network for Internet {
    async fn resolve(&self, name: &str) -> io::Result<Vec<IpAddr>> {
        let found = tokio::net::lookup_host((name, 0)).await?;
        Ok(found.map(|addr| addr.ip()).collect())
    }

    async fn connect(&self, to: SocketAddr) -> io::Result<TcpStream> {
        TcpStream::connect(to).await
    }
}
