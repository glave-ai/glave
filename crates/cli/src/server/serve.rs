//! Serving as the person's server, once it holds the lock: the session API on the person's
//! socket and over HTTP, with what runs in it on its own (`specs/one-server.md`): the tasks a
//! server before it left running recorded as interrupted, the scheduler, and the daily work
//! (this machine's worktrees checked, memory tidied and its keys rotated). Until `SIGTERM` or
//! `SIGINT` (`glave server stop`, Ctrl-C in the foreground, logging out); then it takes no new
//! requests and starts nothing new, gives running turns and tasks 10 seconds to end, interrupts
//! the rest, removes its socket, and its caller lets go of the lock last. Started in the
//! background, it keeps its log under its cap.

use std::pin::pin;
use std::time::Duration;

use anyhow::Context as _;
use glave_harness::Cancel;
use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::{Lock, log, log_path, socket_path};
use crate::args::Args;
use crate::client::Glave;
use crate::data::DataLog;
use crate::memory::Running;

/// How often the server looks at its log's size, and whether the daily work is due.
const TICK: Duration = Duration::from_secs(30);

/// How long running turns and tasks have to end on their own once the server stops.
const GRACE: Duration = Duration::from_secs(10);

/// How often memory is tidied and its keys rotated.
const DAILY: Duration = Duration::from_hours(24);

/// What the server does once a day, beside what it serves.
pub struct Daily<'a> {
    /// The store's log, which the worktrees' check records in.
    pub log: DataLog,
    /// Memory, when it is on.
    pub memory: Option<&'a Running>,
}

impl Daily<'_> {
    /// Runs until it is dropped: this machine's worktrees checked whenever a day has passed
    /// since any glave last checked them, and memory tidied and its keys rotated once a day.
    #[expect(
        clippy::infinite_loop,
        reason = "it runs until dropped; an async fn cannot return `!` on stable Rust"
    )]
    async fn run(&self) {
        let mut tick = tokio::time::interval_at(Instant::now() + TICK, TICK);
        // Both were done as the server started.
        let mut tidied = Instant::now();
        loop {
            tick.tick().await;
            crate::worktrees::check_daily(&self.log).await;
            if tidied.elapsed() < DAILY {
                continue;
            }
            tidied = Instant::now();
            if let Some(memory) = self.memory {
                memory.housekeep().await;
                memory.memory.tidy_soon(&memory.scopes);
            }
        }
    }
}

/// Serves `server` as the person's server, holding its lock, until it is told to stop: first
/// recording the tasks a server before it left running as interrupted, then firing every
/// project's schedules and doing the `daily` work while it runs.
pub async fn run(
    args: &Args,
    server: &Glave,
    daily: &Daily<'_>,
    _held: &Lock,
) -> anyhow::Result<()> {
    let address = args.http.context("the person's server listens over HTTP")?;
    // Bound, and its address written, before the socket answers, so a client of the socket
    // finds it.
    let http = crate::serving::open(args, address).await?;
    let path = socket_path()?;
    // Only the lock's holder removes a socket a server before it left.
    let listener = crate::socket::listen(&path).await?;
    let interrupted = server.recover_tasks().await?;
    if interrupted > 0 {
        eprintln!(
            "glave: {interrupted} tasks a glave before this one left running were interrupted"
        );
    }
    eprintln!("glave: serving you on {}", path.display());
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let stopping = Cancel::new();
    let stop = stopping.clone();
    let mut http = pin!(http.serve(server.clone(), async move { stop.cancelled().await }));
    // Each runs until the loop below ends, and is dropped then. Schedules never fire where
    // people sign in from elsewhere, whose sign-in may end while their schedules last.
    let fires = args.issuers.is_empty();
    let mut scheduler = Box::pin(async move {
        if fires {
            server.run_scheduler(std::future::pending()).await;
        }
        std::future::pending::<()>().await;
    });
    let mut daily = Box::pin(daily.run());
    let mut clients = JoinSet::new();
    // Only a server started in the background writes to the log.
    let log = args.own_session.then(log_path).transpose()?;
    let mut tick = tokio::time::interval_at(Instant::now() + TICK, TICK);
    let ended = loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => serve_client(&mut clients, server, stream),
                Err(error) => break Some(Err(error.into())),
            },
            () = &mut scheduler => {}
            () = &mut daily => {}
            _ = tick.tick() => {
                while clients.try_join_next().is_some() {}
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
    drop((scheduler, daily));
    eprintln!("glave: stopping");
    drop(listener);
    let _ = tokio::fs::remove_file(&path).await;
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

/// Serves `server` to the client at the other end of `stream`, one of `clients`.
fn serve_client(clients: &mut JoinSet<()>, server: &Glave, stream: UnixStream) {
    let (read, write) = stream.into_split();
    let server = server.clone();
    clients.spawn(async move {
        let _ = glave_session::rpc::serve_connection(server, BufReader::new(read), write).await;
    });
}
