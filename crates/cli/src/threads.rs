//! `glave threads`: the threads in this folder, and archiving, unarchiving and deleting them.
//! A change is asked of the glave that is running, when there is one, so that glave stays the
//! one writer of its threads; otherwise it is made straight in the log.

use std::process::ExitCode;

use glave_event::ThreadId;
use glave_harness::{Change, Entropy, OsEntropy, Standing, change_standing, standing_of};
use glave_ports::{EventLog, Scope};
use glave_session::cleanup::{Cleanup, InLog};
use glave_session::protocol::{Reply, Request, Show, code};
use glave_store_sqlite::Store;
use jiff::Timestamp;
use uuid::Uuid;

use crate::local::Peer;
use crate::system::random_uuid;
use crate::{home, keys, worktrees};

pub const USAGE: &str = "\
glave threads: the threads in this folder

Usage:
  glave threads [--archived | --deleted]    list them, newest first: the active ones, or the
                                            archived, or the deleted (waiting for erasure)
  glave threads archive <id>                hide a thread from the list, keeping it
  glave threads unarchive <id>              bring an archived thread back
  glave threads delete <id>                 hide a thread for good and mark it for erasure

Archiving or deleting a thread removes its worktree when that loses nothing (no uncommitted
changes, every commit on a remote) and keeps it, saying why, otherwise. A thread's content is
erased by the operator of the deployment; today's local keys cannot erase one thread alone
(specs/storage.md), so a deleted thread stays in the database, hidden.";

/// What was asked.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    List(Show),
    Change(Change, ThreadId),
}

fn parse(words: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let change = |change, id: &str| {
        let uuid = Uuid::parse_str(id).map_err(|e| format!("{id:?} is not a thread id: {e}"))?;
        let id = ThreadId::try_from(uuid).map_err(|e| format!("{id:?} is not a thread id: {e}"))?;
        Ok(Command::Change(change, id))
    };
    match words.as_slice() {
        [] => Ok(Command::List(Show::Active)),
        ["--archived"] => Ok(Command::List(Show::Archived)),
        ["--deleted"] => Ok(Command::List(Show::Deleted)),
        ["archive", id] => change(Change::Archive, id),
        ["unarchive", id] => change(Change::Unarchive, id),
        ["delete", id] => change(Change::Delete, id),
        [first, ..] => Err(format!("unexpected {first:?}")),
    }
}

pub fn main(arguments: &[String]) -> ExitCode {
    if arguments
        .iter()
        .any(|word| word == "--help" || word == "-h")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let command = match parse(arguments) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("glave: {message}\n\n{USAGE}");
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
    match runtime.block_on(run(command)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> anyhow::Result<ExitCode> {
    let home = home::folder()?;
    let store = Store::open(
        home::database().await?,
        keys::settings(&home, home::relocated()).await?,
    )
    .await?;
    let scope = store
        .local_scope(&std::env::current_dir()?, random_uuid(), random_uuid())
        .await?;
    let log = store.log();
    match command {
        Command::List(show) => list(&log, &scope, show).await,
        Command::Change(change, thread) => apply(&log, &scope, change, thread).await,
    }
}

/// Lists the threads of `show`. Reading needs no writer, so it reads the log itself.
async fn list<L: EventLog>(log: &L, scope: &Scope, show: Show) -> anyhow::Result<ExitCode> {
    let wanted = match show {
        Show::Active => Standing::Active,
        Show::Archived => Standing::Archived,
        Show::Deleted => Standing::Deleted,
    };
    for thread in log.threads(scope, 1_000).await? {
        if standing_of(log, scope, thread).await? == wanted {
            println!("{thread}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Changes `thread`, through the running glave when it knows the thread.
async fn apply<L: EventLog>(
    log: &L,
    scope: &Scope,
    change: Change,
    thread: ThreadId,
) -> anyhow::Result<ExitCode> {
    if let Some(mut peer) = Peer::connect().await {
        let request = match change {
            Change::Archive => Request::ThreadArchive { thread },
            Change::Unarchive => Request::ThreadUnarchive { thread },
            Change::Delete => Request::ThreadDelete { thread },
        };
        match peer.call(&request).await {
            Ok(Reply::Cleaned { notes }) => return Ok(done(change, thread, &notes)),
            Ok(_) => return Ok(done(change, thread, &[])),
            // Not in the running glave's folder: it is made here.
            Err(error) if error.code == code::NO_THREAD => {}
            Err(error) => {
                eprintln!("glave: {error}");
                return Ok(ExitCode::FAILURE);
            }
        }
    }
    let now = Timestamp::now();
    if let Err(error) =
        change_standing(log, scope, thread, now, Entropy::bytes(&OsEntropy), change).await
    {
        eprintln!("glave: {error}");
        return Ok(ExitCode::FAILURE);
    }
    let mut notes = Vec::new();
    if change != Change::Unarchive {
        let cleaning = worktrees::cleaning()?;
        let ledger = InLog(log);
        let trash = cleaning.trash(now);
        let cleanup = Cleanup {
            ledger: &ledger,
            environment: &cleaning.environment,
            trash: trash.as_ref(),
            now,
            entropy: &OsEntropy,
        };
        notes = cleanup
            .archived(&cleaning.list().await?, *scope, thread)
            .await;
    }
    Ok(done(change, thread, &notes))
}

fn done(change: Change, thread: ThreadId, notes: &[String]) -> ExitCode {
    let verb = match change {
        Change::Archive => "archived",
        Change::Unarchive => "unarchived",
        Change::Delete => "deleted",
    };
    println!("{verb} {thread}");
    for note in notes {
        println!("{note}");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_after_threads_pick_a_command() {
        let words = |text: &[&str]| text.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(parse(&words(&[])), Ok(Command::List(Show::Active)));
        assert_eq!(
            parse(&words(&["--archived"])),
            Ok(Command::List(Show::Archived))
        );
        assert!(parse(&words(&["archive"])).is_err());
        assert!(parse(&words(&["archive", "nope"])).is_err());
        let id = "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c13";
        assert!(matches!(
            parse(&words(&["delete", id])),
            Ok(Command::Change(Change::Delete, _))
        ));
    }
}
