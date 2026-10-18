//! Serving the task host: the session API on the folder's Unix socket, one connection per
//! client, with the scheduler running beside it in the one glave that holds the schedules'
//! lock, until nothing is left to do. The person's server listens, recovers tasks and fires
//! schedules the same way, through the pieces here.

use std::fs::File;
use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::{JoinHandle, JoinSet};

use super::locks;
use crate::client::Glave;

/// How often the host looks whether it is still needed, and whether it should fire the
/// schedules.
const IDLE_CHECK: Duration = Duration::from_secs(30);

/// Serves `server` on the socket at `path` until no task runs, no schedule waits for it and no
/// client is connected, or until it is told to stop. When no other host runs, it first records
/// the tasks a host before it left running, in every project, as interrupted.
pub async fn host(server: &Glave, path: &Path) -> anyhow::Result<()> {
    if UnixStream::connect(path).await.is_ok() {
        eprintln!("glave: a task host is already serving {}", path.display());
        return Ok(());
    }
    let listener = listen(path).await?;
    let run = locks::folder()?;
    let _share = join_hosts(server, &run).await?;
    eprintln!("glave: task host serving {}", path.display());
    let outcome = serve(server, &listener, &run).await;
    let _ = tokio::fs::remove_file(path).await;
    outcome
}

/// Listens at `path`, `0600` in a folder private to the person, in place of a socket file a
/// glave that stopped without removing it left there: only for the one glave that may serve
/// it (the task host no other answers for; the person's server, holding its lock).
pub async fn listen(path: &Path) -> anyhow::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt as _;
    crate::socket::private_folder_for(path).await?;
    let _ = tokio::fs::remove_file(path).await;
    let listener = UnixListener::bind(path)
        .with_context(|| format!("could not listen on {}", path.display()))?;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(listener)
}

/// This glave's share of the task hosts' lock in `run`, once it has recorded the tasks one
/// before it left running as interrupted, when no other runs tasks.
pub async fn join_hosts(server: &Glave, run: &Path) -> anyhow::Result<File> {
    crate::socket::private_folder_for(&run.join("hosts.lock")).await?;
    locks::join(run, || async {
        let interrupted = server.recover_tasks().await?;
        if interrupted > 0 {
            eprintln!(
                "glave: {interrupted} tasks a glave before this one left running were \
                 interrupted"
            );
        }
        Ok(())
    })
    .await
}

/// The scheduler, while this glave holds the schedules' lock.
pub struct Scheduling {
    _lock: File,
    running: JoinHandle<()>,
}

impl Scheduling {
    /// Takes the schedules' lock in `run` and starts the scheduler, unless another glave fires
    /// them.
    pub fn start(server: &Glave, run: &Path) -> Option<Self> {
        // A lock that cannot be taken now is tried again at the next check.
        let lock = locks::schedules(run).ok().flatten()?;
        let server = server.clone();
        let running =
            tokio::spawn(async move { server.run_scheduler(std::future::pending()).await });
        Some(Self {
            _lock: lock,
            running,
        })
    }

    /// Stops the scheduler, and lets go of the lock.
    pub async fn stop(self) {
        self.running.abort();
        let _ = self.running.await;
    }
}

/// Serves `server` to the client at the other end of `stream`, one of `clients`.
pub fn serve_client(clients: &mut JoinSet<()>, server: &Glave, stream: UnixStream) {
    let (read, write) = stream.into_split();
    let server = server.clone();
    clients.spawn(async move {
        let _ = glave_session::rpc::serve_connection(server, BufReader::new(read), write).await;
    });
}

async fn serve(server: &Glave, listener: &UnixListener, run: &Path) -> anyhow::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut clients = JoinSet::new();
    let mut scheduling = Scheduling::start(server, run);
    let mut idle = tokio::time::interval_at(tokio::time::Instant::now() + IDLE_CHECK, IDLE_CHECK);
    let outcome = loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => serve_client(&mut clients, server, stream),
                Err(error) => break Err(error.into()),
            },
            _ = idle.tick() => {
                while clients.try_join_next().is_some() {}
                // The glave that fired the schedules ended: this one takes over.
                if scheduling.is_none() {
                    scheduling = Scheduling::start(server, run);
                }
                let schedules_wait = scheduling.is_some()
                    && server.has_schedules().await.unwrap_or(true);
                let needed = !clients.is_empty() || server.running_tasks() > 0 || schedules_wait;
                if !needed {
                    eprintln!("glave: nothing left to run; the task host stops");
                    break Ok(());
                }
            }
            _ = terminate.recv() => break Ok(()),
            _ = interrupt.recv() => break Ok(()),
        }
    };
    if let Some(scheduling) = scheduling {
        scheduling.stop().await;
    }
    outcome
}
