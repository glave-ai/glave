//! `glave server status|stop|restart|link`: the person's server as a process, through its
//! socket and a signal. A stop is graceful (`SIGTERM`, as logging out sends); a restart stops
//! it and starts it again from this shell, so it takes this shell's settings. `link` asks it
//! for a setup link that makes a passkey on a device elsewhere: having a shell here is what
//! proves the person may, so it is also how someone who lost every device gets back in.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use glave_session::protocol::{Reply, Request, TaskStatus};
use rustix::process::{Pid, Signal};

use super::{Lock, connect, log_path, socket_path, start};
use crate::socket::Connection;

const USAGE: &str = "\
glave server: the glave that runs for you on this computer, in the background

Usage:
  glave server status    whether it runs, its version and address, and the tasks running
                         (exits 3 when it does not run)
  glave server stop      stop it, gracefully: running turns and tasks have 10 seconds to
                         finish, and are interrupted after that
  glave server restart   stop it and start it again from this terminal, with its settings
  glave server link      a link that makes a passkey on a device you sign in from elsewhere,
                         at the address origin or domain under [serve] in config.toml
                         names: it works once, for ten minutes. Run it again for another
                         device, or when you have lost every one";

/// How long a stopping server has to end.
const STOP_WAIT: Duration = Duration::from_secs(30);
/// How often a stopping server is looked for.
const STOP_POLL: Duration = Duration::from_millis(50);

/// What `glave server status` exits with when it does not run, as `systemctl status` does.
const NOT_RUNNING: u8 = 3;

/// `glave server …`, reading the words after `server`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("glave: could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let ran = match words.as_slice() {
        ["status"] => runtime.block_on(status()),
        ["stop"] => runtime.block_on(stop()).map(|()| ExitCode::SUCCESS),
        ["restart"] => runtime.block_on(restart()),
        ["link"] => runtime.block_on(link()),
        ["--help" | "-h"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!(
                "glave: glave server takes one command: status, stop, restart or link\n\n{USAGE}"
            );
            return ExitCode::from(2);
        }
    };
    match ran {
        Ok(code) => code,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// `glave server status`.
async fn status() -> anyhow::Result<ExitCode> {
    let Some(mut server) = connect().await? else {
        println!("glave is not running");
        return Ok(ExitCode::from(NOT_RUNNING));
    };
    let Reply::Server(info) = server.request(Request::Discover {}).await? else {
        anyhow::bail!("glave answered server/discover with something else");
    };
    let process = server
        .pid()
        .map_or_else(String::new, |pid| format!(" (process {pid})"));
    println!("glave {} is running{process}", info.version);
    let url = std::fs::read_to_string(crate::home::folder()?.join(crate::app::URL_FILE));
    if let Ok(url) = url {
        println!("  in a browser: {}", url.trim());
    }
    if let Some(origin) = &info.origin {
        println!("  from elsewhere: {origin}");
    }
    println!("  socket: {}", socket_path()?.display());
    println!("  log: {}", log_path()?.display());
    let running = running_tasks(&mut server).await?;
    println!("  tasks running: {running}");
    let here = env!("CARGO_PKG_VERSION");
    if info.version != here {
        println!("  this glave is {here}: glave server restart moves to it");
    }
    Ok(ExitCode::SUCCESS)
}

/// How many tasks `server` runs now.
async fn running_tasks(server: &mut Connection) -> anyhow::Result<usize> {
    let Reply::Tasks { tasks } = server.request(Request::TasksList { limit: None }).await? else {
        anyhow::bail!("glave answered tasks/list with something else");
    };
    Ok(tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Running)
        .count())
}

/// `glave server stop`: asks the server to stop, and waits until its process has let go of
/// the lock, which it does last.
async fn stop() -> anyhow::Result<()> {
    let Some(server) = connect().await? else {
        println!("glave is not running");
        return Ok(());
    };
    let pid = server
        .pid()
        .and_then(Pid::from_raw)
        .context("could not tell which process glave is")?;
    drop(server);
    rustix::process::kill_process(pid, Signal::TERM)
        .with_context(|| format!("could not ask glave (process {}) to stop", pid.as_raw_pid()))?;
    println!("glave is stopping: what runs has up to 10 seconds to finish");
    let waited = tokio::time::Instant::now();
    // Gone once its lock is free: the lock outlives everything else it holds, and a process
    // that ended but was not yet reaped still answers a signal.
    while Lock::take().await?.is_none() {
        if waited.elapsed() > STOP_WAIT {
            anyhow::bail!(
                "glave (process {}) did not stop within {} seconds; what it said is in {}",
                pid.as_raw_pid(),
                STOP_WAIT.as_secs(),
                log_path()?.display()
            );
        }
        tokio::time::sleep(STOP_POLL).await;
    }
    println!("glave stopped");
    Ok(())
}

/// `glave server link`: a setup link from the person's server, started from this terminal if
/// it does not run, for a device to sign in from elsewhere with a passkey it makes.
async fn link() -> anyhow::Result<ExitCode> {
    let Some(server) = super::connect_or_start().await? else {
        anyhow::bail!("glave is not running here: start it first, then ask again");
    };
    let Reply::Server(info) = server.request(Request::Discover {}).await? else {
        anyhow::bail!("glave answered server/discover with something else");
    };
    let Some(origin) = info.origin else {
        anyhow::bail!(
            "glave is reached only on this computer, so no device signs in from elsewhere: set \
             origin or domain under [serve] in config.toml, then restart glave"
        );
    };
    let Reply::Link { code } = server.request(Request::SigninSetup {}).await? else {
        anyhow::bail!("glave answered signin/setup with something else");
    };
    println!(
        "Open this on the device you want to sign in from:\n\n  {origin}/#setup={}\n\nIt \
         works once, for ten minutes, and makes a passkey there.",
        code.expose()
    );
    Ok(ExitCode::SUCCESS)
}

/// `glave server restart`: stops the server, if it runs, and starts it from this shell.
async fn restart() -> anyhow::Result<ExitCode> {
    stop().await?;
    let server = start().await?;
    let Reply::Server(info) = server.request(Request::Discover {}).await? else {
        anyhow::bail!("glave answered server/discover with something else");
    };
    println!("glave {} is running", info.version);
    Ok(ExitCode::SUCCESS)
}
