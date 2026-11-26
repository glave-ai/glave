//! A computer across the wire in the tests: this machine's computers served over an in-memory
//! pipe, and a workspace on one of them reached through a client of them, or one of them as a
//! person's own computer.

use std::io;
use std::path::Path;
use std::sync::Arc;

use glave_computer::{LocalComputers, RemoteComputers, serve};
use glave_harness::Workspace;
use glave_ports::computer::{Computers, Environment, Home, Sources};
use tokio::io::BufReader;
use tokio::task::JoinHandle;

/// `computers` served across the wire, and a workspace on a new computer of them, opened at
/// `folder` through the wire. The server runs until its handle is aborted.
pub(crate) async fn across_the_wire(
    computers: Arc<LocalComputers>,
    folder: &Path,
) -> (Workspace, JoinHandle<io::Result<()>>) {
    let (home, server) = home_across_the_wire(computers, folder).await;
    (Workspace::new(home.computers, home.home), server)
}

/// `computers` served across the wire, and a new computer of them as a person's own, its home
/// opened at `folder` through the wire. The server runs until its handle is aborted.
pub(crate) async fn home_across_the_wire(
    computers: Arc<LocalComputers>,
    folder: &Path,
) -> (Home, JoinHandle<io::Result<()>>) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (read, write) = tokio::io::split(server);
    let server = tokio::spawn(serve(computers, BufReader::new(read), write, None));
    let (read, write) = tokio::io::split(client);
    let remote = RemoteComputers::connect(read, write);
    remote.check_protocol().await.unwrap();
    let computer = remote.create(&Environment::host()).await.unwrap();
    let sources = Sources::Folder {
        path: folder.to_string_lossy().into_owned(),
    };
    let home = remote.open(computer.id, &sources).await.unwrap();
    let computers = Arc::new(remote);
    (Home { computers, home }, server)
}
