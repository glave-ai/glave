//! `glave worktrees`: the worktrees threads made on this machine (with `worktree = true` or
//! `--worktree`), and cleaning them up: by hand with `prune`, and once a day when glave starts.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glave_computer::worktrees::Trash;
use glave_event::{Body, Opened, ThreadId, decode};
use glave_harness::OsEntropy;
use glave_ports::EventLog;
use glave_ports::secrets::without_secrets;
pub use glave_session::cleanup::FOLDER;
use glave_session::cleanup::{Cleaning, Cleanup, InLog, Ledger, is_due};
use glave_session::protocol::{Reply, Request, WorktreeInfo};
use glave_store_sqlite::Store;
use jiff::Timestamp;

use crate::client::Glave;
use crate::local::Peer;
use crate::{home, keys};

pub const USAGE: &str = "\
glave worktrees: the worktrees threads made on this machine

Usage:
  glave worktrees                          list them: folder, branch, whether removing each
                                           would lose work, and any held by a cleanup
  glave worktrees prune [--force <name>]   remove every one that is safe to remove; say why
                                           each other one stays. --force removes the one
                                           named (by its folder's name) anyway

Safe means no uncommitted changes and every commit on a remote, so nothing is lost; a
branch is never deleted. A worktree whose repository is gone goes to the Trash. Once a day,
as glave starts, worktrees whose thread has done nothing for 30 days are removed when safe
and held otherwise.

A thread works in a worktree of its own with --worktree, or worktree = true in config.toml.";

pub fn main(arguments: &[String]) -> ExitCode {
    let mut forced = Vec::new();
    let prune = match arguments.split_first() {
        None => false,
        Some((first, rest)) if first == "prune" => match forced_names(rest) {
            Ok(names) => {
                forced = names;
                true
            }
            Err(message) => {
                eprintln!("glave: {message}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        },
        Some((first, _)) if first == "--help" || first == "-h" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some((first, _)) => {
            eprintln!("glave: unknown worktrees command {first:?}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("glave: could not start: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(prune, &forced)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The names after `prune`: each `--force <name>`.
fn forced_names(words: &[String]) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut words = words.iter();
    while let Some(word) = words.next() {
        match (word.as_str(), words.next()) {
            ("--force", Some(name)) => names.push(name.clone()),
            ("--force", None) => return Err("--force needs a worktree's name".into()),
            (other, _) => return Err(format!("unexpected {other:?} after prune")),
        }
    }
    Ok(names)
}

/// Where this machine's worktrees are kept and what cleaning them needs.
pub fn cleaning() -> anyhow::Result<Cleaning> {
    Ok(Cleaning {
        base: home::folder()?.join(FOLDER),
        environment: without_secrets(std::env::vars_os()),
        home: std::env::var_os("HOME").map(PathBuf::from),
        data_home: std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
    })
}

async fn run(prune: bool, forced: &[String]) -> anyhow::Result<()> {
    let cleaning = cleaning()?;
    // A running glave is the one writer of its threads, so it does the work when there is one.
    let mut peer = Peer::connect().await;
    if !prune {
        let shown = match &mut peer {
            Some(peer) => match peer.call(&Request::WorktreesList {}).await? {
                Reply::Worktrees { worktrees } => worktrees,
                other => anyhow::bail!("the running glave answered worktrees/list with {other:?}"),
            },
            None => cleaning
                .list()
                .await?
                .iter()
                .map(glave_session::worktree_info)
                .collect(),
        };
        print!("{}", listing(&shown, &cleaning.base));
        return Ok(());
    }
    let lines = if let Some(peer) = &mut peer {
        let request = Request::WorktreesPrune {
            force: forced.to_vec(),
        };
        match peer.call(&request).await? {
            Reply::Cleaned { notes } => notes,
            other => anyhow::bail!("the running glave answered worktrees/prune with {other:?}"),
        }
    } else {
        let home = home::folder()?;
        let store = Store::open(
            home::database().await?,
            keys::settings(&home, home::relocated()).await?,
        )
        .await?;
        let log = store.log();
        let now = Timestamp::now();
        let trash = cleaning.trash(now);
        let ledger = InLog(&log);
        let cleanup = in_process(&ledger, &cleaning, trash.as_ref(), now);
        cleanup.prune(&cleaning.list().await?, forced).await
    };
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// A cleanup over `ledger` (the log itself, when no glave is running) with this machine's
/// worktree settings.
fn in_process<'a, T: Ledger>(
    ledger: &'a T,
    cleaning: &'a Cleaning,
    trash: Option<&'a Trash>,
    now: Timestamp,
) -> Cleanup<'a, T> {
    Cleanup {
        ledger,
        environment: &cleaning.environment,
        trash,
        now,
        entropy: &OsEntropy,
    }
}

/// Checks this machine's worktrees as glave starts, at most once a day, with what `log`
/// holds, or by asking a glave that is already running; says what it did on standard error.
/// Never stops glave from starting.
pub async fn check_at_start<L: EventLog>(log: &L) {
    let Ok(home) = home::folder() else {
        return;
    };
    let Ok(cleaning) = cleaning() else {
        return;
    };
    let now = Timestamp::now();
    match is_due(&home, now).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(message) => {
            eprintln!("glave: {message}");
            return;
        }
    }
    let lines = if let Some(mut peer) = Peer::connect().await {
        match peer.call(&Request::WorktreesIdle {}).await {
            Ok(Reply::Cleaned { notes }) => notes,
            Ok(_) => Vec::new(),
            Err(error) => vec![format!("could not check worktrees: {error}")],
        }
    } else {
        match cleaning.list().await {
            Ok(kept) => {
                let trash = cleaning.trash(now);
                let ledger = InLog(log);
                in_process(&ledger, &cleaning, trash.as_ref(), now)
                    .idle(&kept)
                    .await
            }
            Err(error) => vec![format!("could not check worktrees: {error}")],
        }
    };
    for line in lines {
        eprintln!("glave: {line}");
    }
}

/// One line per worktree: its folder, branch, and whether removing it would lose work.
fn listing(shown: &[WorktreeInfo], base: &Path) -> String {
    if shown.is_empty() {
        return format!("no worktrees in {}\n", base.display());
    }
    let mut out = String::new();
    for worktree in shown {
        let standing = worktree
            .at_risk
            .clone()
            .unwrap_or_else(|| "safe to remove".to_owned());
        let held = worktree
            .held
            .as_deref()
            .map(|why| format!("; held ({why})"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{}  {}  ({standing}{held})",
            worktree.path,
            worktree.branch.as_deref().unwrap_or("(no branch)"),
        );
    }
    out
}

/// Where `thread` works, when it is a worktree: for the person to see before the first turn.
pub async fn note(server: &Glave, thread: ThreadId) -> Option<String> {
    let page = Request::ThreadEvents {
        thread,
        after: None,
        limit: Some(16),
    };
    let Ok(Reply::Events { events, .. }) = server.request(page).await else {
        return None;
    };
    events
        .into_iter()
        .find_map(|raw| match decode(raw).ok()?.body {
            Body::WorkspaceBound(bound) => match bound.opened {
                Opened::Worktree { branch, .. } => {
                    Some(format!("working in {} on {branch}", bound.root))
                }
                Opened::Folder { .. } | Opened::Clone { .. } => None,
            },
            _ => None,
        })
}
