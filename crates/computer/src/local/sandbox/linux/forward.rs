//! glave as the forwarder in a sandboxed command's network namespace of its own: started as
//! `glave __glave-forward <socket> -- <command>…` inside bubblewrap, it listens on a port of
//! the namespace's loopback, carries each connection made there to the outbound proxy's Unix
//! socket outside, and runs the command with the proxy's URL in its variables, ending when it
//! ends, with its code. The namespace has no other way out, so a program that ignores the
//! variables finds no network at all.

use std::ffi::OsString;
use std::io;
use std::net::Ipv4Addr;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use glave_process::Program;
use tokio::net::{TcpListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::proxy::Limits;
use crate::proxy::pump::pump;

/// The variable glave hands the proxy's secret to the forwarder in. The command gets it in the
/// proxy's URL instead.
pub(crate) const SECRET: &str = "GLAVE_PROXY_SECRET";

/// The variables that name the proxy, as programs read them.
const PROXY: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

/// The variables that name what is reached without the proxy: the namespace's own loopback,
/// where only what the command starts listens, and which the proxy refuses.
const NO_PROXY: [&str; 2] = ["NO_PROXY", "no_proxy"];

/// The namespace's own loopback, by every name it goes by.
const LOOPBACK: &str = "localhost,127.0.0.1,::1";

/// Connections carried at once; the rest are closed as they come.
const MOST: usize = 256;

/// How long the proxy's socket has to take a connection: it is on this computer.
const CONNECTS_WITHIN: Duration = Duration::from_secs(10);

/// How long the forwarder waits for its last connections once the command has ended.
const ENDS_WITHIN: Duration = Duration::from_secs(1);

const USAGE: &str = "usage: glave __glave-forward <socket> -- <command>…";

/// Runs the forwarder on its arguments (after `__glave-forward`): the command's exit code once
/// it has ended, a signal that ended it as a shell says it (128 and its number), or what went
/// wrong before it could run.
pub(in crate::local::sandbox) fn forward(args: &[OsString]) -> Result<ExitCode, String> {
    let [socket, separator, program, rest @ ..] = args else {
        return Err(USAGE.into());
    };
    if separator != "--" {
        return Err(USAGE.into());
    }
    let secret = std::env::var(SECRET).map_err(|_| format!("{SECRET} is not set"))?;
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.set_nonblocking(true).map(|()| listener))
        .map_err(|error| format!("could not listen on the namespace's loopback: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start: {error}"))?;
    let url = format!("http://t:{secret}@127.0.0.1:{port}");
    let inherited: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| {
            name != SECRET && !PROXY.iter().chain(&NO_PROXY).any(|own| name == *own)
        })
        .collect();
    let mut command = Program::new(program);
    command
        .args(rest)
        .env_clear()
        .envs(inherited)
        .envs(PROXY.map(|name| (name, url.as_str())))
        .envs(NO_PROXY.map(|name| (name, LOOPBACK)));
    let ran = runtime.block_on(async move {
        let carrying = tokio::spawn(carry(listener, PathBuf::from(socket)));
        let ran = tokio::task::spawn_blocking(move || command.status_blocking()).await;
        carrying.abort();
        ran
    });
    // The command has ended; its connections end with it.
    runtime.shutdown_timeout(ENDS_WITHIN);
    let status = ran
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    let code = status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(u8::MAX);
    Ok(ExitCode::from(code))
}

/// Carries each connection `listener` takes to the proxy's `socket`, as a task it owns, until
/// the forwarder ends.
#[expect(
    clippy::infinite_loop,
    reason = "the forwarder carries connections until its command ends, which aborts this"
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
