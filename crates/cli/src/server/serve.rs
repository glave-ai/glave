//! Serving as the person's server, once it holds the lock: the session API on the person's
//! socket and over HTTP, the scheduler while it holds the schedules' lock, until `SIGTERM` or
//! `SIGINT` (`glave server stop`, Ctrl-C in the foreground, logging out). Then it takes no new
//! requests and starts nothing new, gives running turns and tasks 10 seconds to end, interrupts
//! the rest, removes its socket, and its caller lets go of the lock last. Started in the
//! background, it keeps its log under its cap.

use std::pin::pin;
use std::time::Duration;

use anyhow::Context as _;
use glave_harness::Cancel;
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinSet;

use super::{Lock, log, log_path, socket_path};
use crate::args::Args;
use crate::client::Glave;
use crate::daemon::{self, Scheduling};

/// How often the server takes over the schedules when the glave that fired them ended, and
/// looks at its log's size.
const TICK: Duration = Duration::from_secs(30);

/// How long running turns and tasks have to end on their own once the server stops.
const GRACE: Duration = Duration::from_secs(10);

/// Serves `server` as the person's server, holding its lock, until it is told to stop. When no
/// other glave runs tasks, it first records those one before it left running as interrupted.
pub async fn run(args: &Args, server: &Glave, _held: &Lock) -> anyhow::Result<()> {
    let address = args.http.context("the person's server listens over HTTP")?;
    // Bound, and its address written, before the socket answers, so a client of the socket
    // finds it.
    let http = crate::serving::open(args, address).await?;
    let path = socket_path()?;
    // Only the lock's holder removes a socket a server before it left.
    let listener = daemon::listen(&path).await?;
    let run = daemon::run_folder()?;
    let _share = daemon::join_hosts(server, &run).await?;
    eprintln!("glave: serving you on {}", path.display());
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let stopping = Cancel::new();
    let stop = stopping.clone();
    let mut http = pin!(http.serve(server.clone(), async move { stop.cancelled().await }));
    let mut clients = JoinSet::new();
    let mut scheduling = Scheduling::start(server, &run);
    // Only a server started in the background writes to the log.
    let log = args.own_session.then(log_path).transpose()?;
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
    let ended = loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => daemon::serve_client(&mut clients, server, stream),
                Err(error) => break Some(Err(error.into())),
            },
            _ = tick.tick() => {
                while clients.try_join_next().is_some() {}
                if scheduling.is_none() {
                    scheduling = Scheduling::start(server, &run);
                }
                if let Some(log) = &log
                    && let Err(error) = log::keep_capped(log.clone()).await
                {
                    eprintln!("glave: could not keep the log under its cap: {error}");
                }
            }
            outcome = &mut http => break Some(outcome),
            _ = terminate.recv() => break None,
            _ = interrupt.recv() => break None,
        }
    };
    eprintln!("glave: stopping");
    drop(listener);
    let _ = tokio::fs::remove_file(&path).await;
    if let Some(scheduling) = scheduling.take() {
        scheduling.stop().await;
    }
    // Every turn, the socket's clients' too: what still runs after the grace is interrupted.
    server.stop(GRACE).await;
    stopping.cancel();
    let outcome = match ended {
        Some(outcome) => outcome,
        None => http.await,
    };
    clients.abort_all();
    while clients.join_next().await.is_some() {}
    outcome
}
