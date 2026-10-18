//! The person's server (`specs/one-server.md`, "The person's server"): one per glave folder,
//! the holder of `GLAVE_HOME/run/server.lock` for its whole life, serving the session API on
//! the person's socket (`run/server.sock`) and over loopback HTTP. `glave serve` runs it in the
//! foreground; a command run from the person's own terminal that needs it and finds none
//! starts it in the background, in a session of its own, so neither Ctrl-C nor closing the
//! terminal reaches it. With no one at the terminal nothing starts it, so scripts, CI, tests
//! and `ssh host glave "…"` never leave one behind. It runs until `glave server stop`.

mod command;
mod log;
mod serve;

use std::fs::{File, OpenOptions, TryLockError};
use std::io::IsTerminal as _;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, bail};

use crate::home;
use crate::socket::{self, Connection};

pub use command::main;
pub use serve::run;

/// The port the person's server listens on over HTTP when it is free, so the browser's origin
/// stays the same across restarts.
const PORT: u16 = 4180;

/// How long a new server has to start listening, as the task host's starter waits.
const START_WAIT: Duration = Duration::from_secs(20);
/// How often a starting server is tried.
const START_POLL: Duration = Duration::from_millis(50);

/// What names the server in errors about its socket.
const WHAT: &str = "glave server";

/// `GLAVE_HOME/run`, where the server keeps its lock, socket and log.
fn run_folder() -> anyhow::Result<PathBuf> {
    Ok(home::folder()?.join("run"))
}

/// Where the person's server listens: `run/server.sock`, or where a path too long for a socket
/// moves to.
pub fn socket_path() -> anyhow::Result<PathBuf> {
    socket::place(run_folder()?.join("server.sock"))
}

/// Where a server started in the background writes what it says: `run/server.log`.
pub fn log_path() -> anyhow::Result<PathBuf> {
    Ok(run_folder()?.join("server.log"))
}

/// The person's server's lock, held for its whole life. The system lets go of it when the
/// process ends, however it ends, so a crash leaves no stale lock.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

impl Lock {
    /// The lock, unless another server holds it.
    pub async fn take() -> anyhow::Result<Option<Self>> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let path = run_folder()?.join("server.lock");
        socket::private_folder_for(&path).await?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("could not open {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => {
                Err(error).with_context(|| format!("could not lock {}", path.display()))
            }
        }
    }
}

/// Where the person's server listens over HTTP when nothing else is chosen: 127.0.0.1:4180
/// when it is free, else any free port.
pub fn loopback() -> SocketAddr {
    let usual = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));
    if TcpListener::bind(usual).is_ok() {
        usual
    } else {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
    }
}

/// Whether someone is at this command's terminal: a terminal on its input and its output.
pub fn at_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// The person's server, if one runs: only their own.
pub async fn connect() -> anyhow::Result<Option<Connection>> {
    Connection::open(&socket_path()?, WHAT).await
}

/// The person's server: the one running, or one started now when someone is at the terminal;
/// none when no one is and none runs.
pub async fn connect_or_start() -> anyhow::Result<Option<Connection>> {
    if let Some(server) = connect().await? {
        return Ok(Some(server));
    }
    if !at_terminal() {
        return Ok(None);
    }
    start().await.map(Some)
}

/// Starts the person's server apart from this command and its terminal, from this shell and
/// folder, and waits until it answers. Two commands that start one at once each start a
/// server; one takes the lock, the other exits, and both connect to the one that runs.
pub async fn start() -> anyhow::Result<Connection> {
    let log_path = log_path()?;
    socket::private_folder_for(&log_path).await?;
    let log = {
        use std::os::unix::fs::OpenOptionsExt as _;
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&log_path)
    }
    .with_context(|| format!("could not open {}", log_path.display()))?;
    let glave = std::env::current_exe().context("could not find this glave to start it")?;
    // It starts a session of its own first thing (`serve --own-session`), so it outlives this
    // command on purpose and owns what it starts.
    glave_process::Program::new(glave)
        .args(["serve", "--own-session"])
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .starts_own_session()
        .detach()
        .context("could not start glave in the background")?;
    let waited = tokio::time::Instant::now();
    loop {
        if let Some(server) = connect().await? {
            return Ok(server);
        }
        if waited.elapsed() > START_WAIT {
            bail!(
                "glave did not start in the background; what it said is in {}",
                log_path.display()
            );
        }
        tokio::time::sleep(START_POLL).await;
    }
}

/// For `glave serve` as the person's server (`args`), before anything starts, so a second one
/// starts nothing: a session of its own when started in the background, so closing the
/// terminal that started it or Ctrl-C there never reaches it; then the lock, refused when
/// another server holds it; and loopback HTTP when nothing else is chosen. No lock for a
/// program's own server or a task host.
pub async fn claim(args: &mut crate::args::Args) -> anyhow::Result<Option<Lock>> {
    if !args.person_s_server() {
        return Ok(None);
    }
    if args.own_session {
        rustix::process::setsid().context("could not start a session of its own")?;
    }
    let Some(lock) = Lock::take().await? else {
        bail!(
            "glave already runs for {} (glave server status says where); this one stops",
            home::folder()?.display()
        );
    };
    args.http.get_or_insert_with(loopback);
    Ok(Some(lock))
}
