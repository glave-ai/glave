//! glave as the forwarder in a thread's network namespace (`namespace.rs`): started as
//! `glave __glave-forward <socket>` once it has joined the namespace (`enter.rs`), it listens on
//! a port of the namespace's loopback, says that port on a line of its standard output, and
//! carries each connection made there to the outbound proxy's Unix socket, until glave stops
//! it. The thread's commands join the namespace too, so the proxy is their one way out, and a
//! program that ignores the proxy's variables finds no network at all.

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::proxy::Limits;
use crate::proxy::pump::pump;

/// Connections carried at once; the rest are closed as they come.
const MOST: usize = 256;

/// How long the proxy's socket has to take a connection: it is on this computer.
const CONNECTS_WITHIN: Duration = Duration::from_secs(10);

const USAGE: &str = "usage: glave __glave-forward <socket>";

/// Runs the forwarder on its arguments (after `__glave-forward`) until it is stopped. Returns
/// only when it could not start, saying why.
pub(in crate::local::sandbox) fn forward(args: &[OsString]) -> String {
    let [socket] = args else {
        return USAGE.into();
    };
    let listener = match std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
    {
        Ok(listener) => listener,
        Err(error) => return format!("could not listen on the namespace's loopback: {error}"),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return format!("could not start: {error}"),
    };
    if let Err(error) = say_ready(&listener) {
        return format!("could not say where it listens: {error}");
    }
    runtime.block_on(carry(listener, PathBuf::from(socket)));
    "could not carry connections from the namespace's loopback".into()
}

/// Says the port `listener` has on a line of standard output: the thread's commands reach the
/// proxy there.
fn say_ready(listener: &std::net::TcpListener) -> io::Result<()> {
    let port = listener.local_addr()?.port();
    let mut said = io::stdout();
    writeln!(said, "{port}")?;
    said.flush()
}

/// Carries each connection `listener` takes to the proxy's `socket`, as a task it owns, until
/// the forwarder is stopped.
#[expect(
    clippy::infinite_loop,
    reason = "the forwarder carries connections until glave stops it"
)]
async fn carry(listener: std::net::TcpListener, socket: PathBuf) {
    let Ok(listener) = TcpListener::from_std(listener) else {
        return;
    };
    let places = Arc::new(Semaphore::new(MOST));
    let idle = Limits::default().idle;
    let mut carried = JoinSet::new();
    loop {
        while carried.try_join_next().is_some() {}
        let Ok((client, _)) = listener.accept().await else {
            // Out of files, say: let the connections it has end before taking more.
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        // Over the cap, the connection is closed as it is dropped.
        let Ok(place) = Arc::clone(&places).try_acquire_owned() else {
            continue;
        };
        let socket = socket.clone();
        carried.spawn(async move {
            let _place = place;
            let _ = carry_one(client, &socket, idle).await;
        });
    }
}

/// Carries one connection to the proxy, both ways, until both sides are done or nothing has
/// moved for `idle`.
async fn carry_one(
    mut client: tokio::net::TcpStream,
    socket: &Path,
    idle: Duration,
) -> io::Result<()> {
    let mut proxy = timeout(CONNECTS_WITHIN, UnixStream::connect(socket))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    pump(&mut client, &mut proxy, idle).await
}
