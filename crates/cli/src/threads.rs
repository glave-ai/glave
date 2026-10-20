//! `glave threads`: the threads in this folder, and archiving, unarchiving and deleting them,
//! through the person's server when it runs (or a terminal starts it). Where none runs and no
//! one is at the terminal, a change is made straight in the log; a glave holding the thread
//! takes it in before its next event, so the two never fight.

use std::process::ExitCode;

use glave_event::ThreadId;
use glave_harness::{Change, change_standing};
use glave_ports::{
    Clock as _, Entropy, EventLog, OsEntropy, Scope, Standing, SystemClock, ThreadQuery,
};
use glave_session::protocol::{Reply, Request, Show};
use uuid::Uuid;

use crate::reach::Reached;
use crate::worktrees;

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
    match crate::server::connect_or_start().await? {
        Some(server) => through(&mut Reached::Server(server), command).await,
        None => in_the_log(command).await,
    }
}

/// `command` through the person's server; worktrees released here, as `glave worktrees` does.
async fn through(server: &mut Reached, command: Command) -> anyhow::Result<()> {
    let thread = match command {
        Command::List(standing) => {
            let show = match standing {
                Standing::Active => Show::Active,
                Standing::Archived => Show::Archived,
                Standing::Deleted => Show::Deleted,
            };
            let listed = Request::ThreadList {
                project: Some(server.project(&std::env::current_dir()?).await?),
                limit: Some(MOST_LISTED),
                show: Some(show),
                cursor: None,
            };
            let Reply::Threads { threads, .. } = server.request(listed).await? else {
                anyhow::bail!("glave answered thread/list with something else");
            };
            for thread in &threads {
                println!("{}", thread.id);
            }
            return Ok(());
        }
        Command::Change(change, thread) => {
            let request = match change {
                Change::Archive => Request::ThreadArchive { thread },
                Change::Unarchive => Request::ThreadUnarchive { thread },
                Change::Delete => Request::ThreadDelete { thread },
            };
            server.request(request).await?;
            println!("{} {thread}", done(change));
            if change == Change::Unarchive {
                return Ok(());
            }
            thread
        }
    };
    // The change is made; a worktree that could not be looked at stays, and is said.
    match released(thread).await {
        Ok(lines) => lines.iter().for_each(|line| println!("{line}")),
        Err(error) => println!("its worktree, if it has one, is kept: {error:#}"),
    }
    Ok(())
}

/// Releases the worktrees of `thread`, just archived or deleted: what became of each.
async fn released(thread: ThreadId) -> anyhow::Result<Vec<String>> {
    let store = crate::data::Data::here().await?;
    let person = crate::identity::load_or_make(&store).await?.person;
    let log = store.log();
    let Some(located) = log.locate(thread).await? else {
        return Ok(Vec::new());
    };
    let Some(project) = store.projects().get(located.org, located.project).await? else {
        return Ok(Vec::new());
    };
    // Only a thread of a project the person here works in.
    match Scope::of_member(located.org, &project, person) {
        Some(scope) => worktrees::release_archived(&log, scope, thread).await,
        None => Ok(Vec::new()),
    }
}

/// `command` made straight in the log.
async fn in_the_log(command: Command) -> anyhow::Result<()> {
    let store = crate::data::Data::here().await?;
    let person = crate::identity::load_or_make(&store).await?.person;
    let scope = crate::projects::here(&store, &std::env::current_dir()?, person).await?;
    let log = store.log();
    match command {
        Command::List(standing) => {
            let query = ThreadQuery {
                standing,
                limit: MOST_LISTED,
                after: None,
            };
            for view in log.thread_views(&scope, &query).await? {
                if glave_ports::stream::Reserved::of(view.thread).is_none() {
                    println!("{}", view.thread);
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
    let now = SystemClock.now();
    change_standing(log, &scope, thread, now, OsEntropy.bytes()?, change).await?;
    println!("{} {thread}", done(change));
    if change != Change::Unarchive {
        for line in worktrees::release_archived(log, scope, thread).await? {
            println!("{line}");
        }
    }
    Ok(())
}

/// What `change` did, as said.
fn done(change: Change) -> &'static str {
    match change {
        Change::Archive => "archived",
        Change::Unarchive => "unarchived",
        Change::Delete => "deleted",
    }
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
