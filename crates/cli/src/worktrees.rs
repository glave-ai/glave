//! `glave worktrees`: the worktrees threads made on this machine (with `worktree = true` or
//! `--worktree`), and cleaning them up: by hand with `prune`, and once a day when glave starts.

mod cleanup;

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use glave_computer::worktrees::{self, Kept, Trash};
use glave_event::{Body, Opened, ThreadId, decode};
use glave_harness::OsEntropy;
use glave_ports::secrets::without_secrets;
use glave_ports::{EventLog, Scope};
use glave_session::protocol::{Reply, Request};
use jiff::{SignedDuration, Timestamp};

use crate::client::Glave;
use crate::home;

pub use cleanup::Cleanup;

/// The folder in glave's folder where threads' worktrees are kept, one folder per repository.
pub const FOLDER: &str = "worktrees";

/// The folder in glave's folder where threads with no project get a scratch folder each.
pub const SCRATCH: &str = "scratch";

/// The file in glave's folder holding when worktrees were last checked.
const SWEPT: &str = "worktrees-checked";

/// How often glave checks worktrees as it starts.
const EVERY: SignedDuration = SignedDuration::from_hours(24);

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

/// The worktrees and scratch folders kept under `home`.
async fn kept(home: &Path, environment: &[(OsString, OsString)]) -> anyhow::Result<Vec<Kept>> {
    let mut kept = worktrees::list(&home.join(FOLDER), environment).await?;
    kept.extend(worktrees::list_scratch(&home.join(SCRATCH)).await?);
    Ok(kept)
}

async fn run(prune: bool, forced: &[String]) -> anyhow::Result<()> {
    let home = home::folder()?;
    let environment = without_secrets(std::env::vars_os());
    let kept = kept(&home, &environment).await?;
    if !prune {
        print!("{}", listing(&kept, &home.join(FOLDER)));
        return Ok(());
    }
    let store = crate::data::Data::here().await?;
    let now = Timestamp::now();
    let trash = this_machines_trash(now);
    let cleanup = Cleanup {
        log: &store.log(),
        environment: &environment,
        trash: trash.as_ref(),
        now,
        entropy: &OsEntropy,
    };
    for line in cleanup.prune(&kept, forced).await {
        println!("{line}");
    }
    Ok(())
}

/// Releases the worktrees of `thread` in `scope`, which was just archived or deleted, with
/// what `log` holds: one line for each, saying what became of it.
pub async fn release_archived<L: EventLog>(
    log: &L,
    scope: Scope,
    thread: ThreadId,
) -> anyhow::Result<Vec<String>> {
    let environment = without_secrets(std::env::vars_os());
    let kept = kept(&home::folder()?, &environment).await?;
    let now = Timestamp::now();
    let trash = this_machines_trash(now);
    let cleanup = Cleanup {
        log,
        environment: &environment,
        trash: trash.as_ref(),
        now,
        entropy: &OsEntropy,
    };
    Ok(cleanup.archived(&kept, scope, thread).await)
}

/// The person's Trash on this machine, if it has one glave knows.
fn this_machines_trash(now: Timestamp) -> Option<Trash> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let deleted_at = now.strftime("%Y-%m-%dT%H:%M:%S").to_string();
    Trash::for_home(&home, data_home.as_deref(), &deleted_at)
}

/// Checks this machine's worktrees as glave starts, and in the person's server as it runs, at
/// most once a day, with what `log` holds; says what it did on standard error. Never stops glave.
pub async fn check_daily<L: EventLog>(log: &L) {
    let Ok(home) = home::folder() else {
        return;
    };
    let environment = without_secrets(std::env::vars_os());
    let now = Timestamp::now();
    let trash = this_machines_trash(now);
    let cleanup = Cleanup {
        log,
        environment: &environment,
        trash: trash.as_ref(),
        now,
        entropy: &OsEntropy,
    };
    for line in check_if_due(&home, &cleanup).await {
        eprintln!("glave: {line}");
    }
}

/// The daily check, when the last one was a day or more before `cleanup.now`: it records the
/// time first, so a check that fails is not retried until tomorrow.
pub async fn check_if_due<L: EventLog>(home: &Path, cleanup: &Cleanup<'_, L>) -> Vec<String> {
    let stamp = home.join(SWEPT);
    let last = tokio::fs::read_to_string(&stamp)
        .await
        .ok()
        .and_then(|text| text.trim().parse::<Timestamp>().ok());
    if last.is_some_and(|last| cleanup.now.duration_since(last) < EVERY) {
        return Vec::new();
    }
    let mut any = false;
    for base in [FOLDER, SCRATCH] {
        any |= tokio::fs::try_exists(home.join(base))
            .await
            .unwrap_or(false);
    }
    if !any {
        return Vec::new();
    }
    if let Err(error) = tokio::fs::write(&stamp, cleanup.now.to_string()).await {
        return vec![format!("could not write {}: {error}", stamp.display())];
    }
    match kept(home, cleanup.environment).await {
        Ok(kept) => cleanup.idle(&kept).await,
        Err(error) => vec![format!("could not check worktrees: {error}")],
    }
}

/// One line per worktree: its folder, branch, and whether removing it would lose work.
fn listing(kept: &[Kept], base: &Path) -> String {
    if kept.is_empty() {
        return format!("no worktrees in {}\n", base.display());
    }
    let mut out = String::new();
    for kept in kept {
        let standing = match kept.at_risk() {
            None => "safe to remove".to_owned(),
            Some(risk) => risk,
        };
        let held = kept
            .held
            .as_deref()
            .map(|why| format!("; held ({why})"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{}  {}  ({standing}{held})",
            kept.path.display(),
            kept.branch.as_deref().unwrap_or("(no branch)"),
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
                Opened::Scratch { path } => Some(format!("working in a scratch folder, {path}")),
                Opened::Folder { .. } | Opened::Clone { .. } => None,
            },
            _ => None,
        })
}
