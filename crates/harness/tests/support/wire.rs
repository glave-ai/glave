//! A computer across the wire in the tests: this machine's computers served over TCP on
//! 127.0.0.1, and a workspace on one of them reached through a client signed in to them.

use std::io;
use std::path::Path;
use std::sync::Arc;

use glave_computer::{LocalComputers, RemoteComputers, serve_tcp};
use glave_harness::Workspace;
use glave_ports::computer::{Computers, Environment, Sources};
use tokio::task::JoinHandle;

/// `computers` served over TCP, and a workspace on a new computer of them, opened at `folder`
/// through the wire. The server runs until its handle is aborted.
pub(crate) async fn across_the_wire(
    computers: Arc<LocalComputers>,
    folder: &Path,
) -> (Workspace, JoinHandle<io::Result<()>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(serve_tcp(computers, listener, "token".into()));
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let (read, write) = stream.into_split();
    let remote = RemoteComputers::connect(read, write);
    remote.authenticate("token").await.unwrap();
    remote.check_protocol().await.unwrap();
    let computer = remote.create(&Environment::host()).await.unwrap();
    let sources = Sources::Folder {
        path: folder.to_string_lossy().into_owned(),
    };
    let info = remote.open(computer.id, &sources).await.unwrap();
    (Workspace::new(Arc::new(remote), info), server)
}
