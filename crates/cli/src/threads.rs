//! `glave threads`: the threads in this folder, and archiving, unarchiving and deleting them.
//! A change is made straight in the log; a glave holding the thread takes it in before its
//! next event, so the two never fight.

use std::process::ExitCode;

use glave_event::ThreadId;
use glave_harness::{Change, Entropy, OsEntropy, Standing, change_standing, standing_of};
use glave_ports::{EventLog, Scope};
use jiff::Timestamp;
use uuid::Uuid;

use crate::{home, worktrees};

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

/// The most threads listed.
const MOST_LISTED: usize = 1_000;

/// What was asked.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    List(Standing),
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
        [] => Ok(Command::List(Standing::Active)),
        ["--archived"] => Ok(Command::List(Standing::Archived)),
        ["--deleted"] => Ok(Command::List(Standing::Deleted)),
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
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> anyhow::Result<()> {
    let home = home::folder()?;
    let store = crate::data::Data::here().await?;
    let person = crate::identity::load_or_make(&home).await?.person;
    let scope = crate::spaces::here(&store, &std::env::current_dir()?, person).await?;
    let log = store.log();
    match command {
        Command::List(wanted) => {
            for thread in log.threads(&scope, MOST_LISTED).await? {
                if glave_session::is_ledger(thread) {
                    continue;
                }
                if standing_of(&log, &scope, thread).await? == wanted {
                    println!("{thread}");
                }
            }
            Ok(())
        }
        Command::Change(change, thread) => apply(&log, scope, change, thread).await,
    }
}

/// Changes `thread`, and releases its worktrees when it is archived or deleted.
async fn apply<L: EventLog>(
    log: &L,
    scope: Scope,
    change: Change,
    thread: ThreadId,
) -> anyhow::Result<()> {
    let now = Timestamp::now();
    change_standing(log, &scope, thread, now, OsEntropy.bytes(), change).await?;
    let verb = match change {
        Change::Archive => "archived",
        Change::Unarchive => "unarchived",
        Change::Delete => "deleted",
    };
    println!("{verb} {thread}");
    if change != Change::Unarchive {
        for line in worktrees::release_archived(log, scope, thread).await? {
            println!("{line}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_after_threads_pick_a_command() {
        let words = |text: &[&str]| text.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        assert_eq!(parse(&words(&[])), Ok(Command::List(Standing::Active)));
        assert_eq!(
            parse(&words(&["--archived"])),
            Ok(Command::List(Standing::Archived))
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
