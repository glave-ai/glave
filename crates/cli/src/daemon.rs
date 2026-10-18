//! The task host: the long-running glave for one folder that runs the tasks started from it,
//! so they go on after the command that started them exits, and, one host at a time, every
//! project's schedules. It is `glave serve --socket`, serving the session API on a Unix socket
//! in GLAVE_HOME that only the person can open. Commands that need it (`--background`,
//! `glave tasks`, `glave schedule`) start it when it is not running, and it exits on its own
//! once nothing runs, no schedule waits for it and no one is connected. It goes once the
//! person's server (`crate::server`) runs tasks and schedules alone (`specs/one-server.md`, 2c).

mod host;
mod locks;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, bail};
use glave_session::protocol::{Reply, Request};

use crate::args::Args;
use crate::home;
use crate::socket::{self, Connection};

pub use host::{Scheduling, host, join_hosts, listen, serve_client};
pub use locks::folder as run_folder;

/// Whether a task host, or the person's server, fires every project's schedules now.
pub fn schedules_fired() -> bool {
    locks::folder().is_ok_and(|run| locks::schedules_fired(&run))
}

/// How long a new host has to start listening.
const START_WAIT: Duration = Duration::from_secs(20);
/// How often a starting host is tried.
const START_POLL: Duration = Duration::from_millis(50);

/// Where the host for `folder` keeps its socket name, log and marks: `GLAVE_HOME/run/<hash of
/// the folder>`, with an extension.
fn run_file(folder: &Path, extension: &str) -> anyhow::Result<PathBuf> {
    Ok(home::folder()?.join("run").join(format!(
        "{:016x}.{extension}",
        socket::fnv1a(folder.as_os_str().as_encoded_bytes())
    )))
}

/// Where the host for `folder` listens: `GLAVE_HOME/run/<hash of the folder>.sock`, or where
/// `socket::place` moves a path too long for a socket.
pub fn socket(folder: &Path) -> anyhow::Result<PathBuf> {
    socket::place(run_file(folder, "sock")?)
}

/// A connection to the task host.
pub struct Host(Connection);

impl Host {
    /// The host for `folder`, if one is running: only the person's own.
    pub async fn connect(folder: &Path) -> anyhow::Result<Option<Self>> {
        Ok(Connection::open(&socket(folder)?, "task host")
            .await?
            .map(Self))
    }

    /// The host for `folder`, started with the settings `args` choose when none is running.
    pub async fn connect_or_start(folder: &Path, args: &Args) -> anyhow::Result<Self> {
        if let Some(host) = Self::connect(folder).await? {
            return Ok(host);
        }
        start(folder, args).await?;
        let waited = tokio::time::Instant::now();
        loop {
            if let Some(host) = Self::connect(folder).await? {
                return Ok(host);
            }
            if waited.elapsed() > START_WAIT {
                bail!(
                    "the task host did not start; what it printed is in {}",
                    log_file(folder)?.display()
                );
            }
            tokio::time::sleep(START_POLL).await;
        }
    }

    /// Sends `request` and answers with the reply, or the error the host gave.
    pub async fn request(&mut self, request: Request) -> anyhow::Result<Reply> {
        self.0.request(request).await
    }
}

/// Where the host for `folder` writes what it prints.
pub fn log_file(folder: &Path) -> anyhow::Result<PathBuf> {
    run_file(folder, "log")
}

/// Where a session here remembers which finished tasks it has told the person about.
pub fn seen_file(folder: &Path) -> anyhow::Result<PathBuf> {
    run_file(folder, "seen")
}

/// Starts the host for `folder` on its own, apart from this process and its terminal.
async fn start(folder: &Path, args: &Args) -> anyhow::Result<()> {
    socket::private_folder_for(&log_file(folder)?).await?;
    socket::private_folder_for(&socket(folder)?).await?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file(folder)?)?;
    let glave = std::env::current_exe().context("could not find this glave to start the host")?;
    // Apart from this process, in a group of its own, so Ctrl-C in this terminal does not reach
    // it: it outlives this command on purpose, owns what it starts, and exits once idle.
    glave_process::Program::new(glave)
        .args(["serve", "--socket"])
        .args(settings(args))
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .detach()
        .context("could not start the task host")?;
    Ok(())
}

/// The options a new host takes from this command: the settings its tasks run with.
fn settings(args: &Args) -> Vec<String> {
    let mut words = Vec::new();
    if let Some(model) = &args.model {
        words.extend(["--model".to_owned(), model.clone()]);
    }
    if let Some(approval) = args.approval {
        words.extend(["--approval".to_owned(), approval.name().to_owned()]);
    }
    if let Some(sandbox) = args.sandbox {
        words.extend(["--sandbox".to_owned(), sandbox.name().to_owned()]);
    }
    match args.network {
        Some(true) => words.push("--network".to_owned()),
        Some(false) => words.push("--no-network".to_owned()),
        None => {}
    }
    if let Some(on) = args.on {
        words.extend(["--on".to_owned(), on.name().to_owned()]);
    }
    if let Some(steps) = args.max_steps {
        words.extend(["--max-steps".to_owned(), steps.to_string()]);
    }
    words
}
