//! Serving the task host: the session API on the folder's Unix socket, one connection per
//! client, with the scheduler running beside it, until nothing is left to do.

use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinSet;

use crate::client::Glave;

/// How often the host looks whether it is still needed.
const IDLE_CHECK: Duration = Duration::from_secs(30);

/// Serves `server` on the socket at `path` until no task runs, no schedule waits and no client
/// is connected, or until it is told to stop. Recovers the folder's tasks that a host before it
/// left running first: they are recorded as interrupted.
pub async fn host(server: &Glave, path: &Path) -> anyhow::Result<()> {
    if UnixStream::connect(path).await.is_ok() {
        eprintln!("glave: a task host is already serving {}", path.display());
        return Ok(());
    }
    super::private_folder_for(path).await?;
    // A socket left by a host that stopped without removing it.
    let _ = tokio::fs::remove_file(path).await;
    let listener = UnixListener::bind(path)
        .with_context(|| format!("could not listen on {}", path.display()))?;
    {
        use std::os::unix::fs::PermissionsExt as _;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    }
    let interrupted = server.recover_tasks().await?;
    if interrupted > 0 {
        eprintln!(
            "glave: {interrupted} tasks a host before this one left running were interrupted"
        );
    }
    eprintln!("glave: task host serving {}", path.display());
    let outcome = serve(server, &listener).await;
    let _ = tokio::fs::remove_file(path).await;
    outcome
}

async fn serve(server: &Glave, listener: &UnixListener) -> anyhow::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut clients = JoinSet::new();
    let scheduler = server.run_scheduler(std::future::pending());
    let mut scheduler = std::pin::pin!(scheduler);
    let mut idle = tokio::time::interval_at(tokio::time::Instant::now() + IDLE_CHECK, IDLE_CHECK);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let (read, write) = stream.into_split();
                let server = server.clone();
                clients.spawn(async move {
                    let _ = glave_session::rpc::serve_connection(server, BufReader::new(read), write)
                        .await;
                });
            }
            () = &mut scheduler => return Ok(()),
            _ = idle.tick() => {
                while clients.try_join_next().is_some() {}
                let needed = !clients.is_empty()
                    || server.running_tasks() > 0
                    || server.has_schedules().await.unwrap_or(true);
                if !needed {
                    eprintln!("glave: nothing left to run; the task host stops");
                    return Ok(());
                }
            }
            _ = terminate.recv() => return Ok(()),
            _ = interrupt.recv() => return Ok(()),
        }
    }
}
