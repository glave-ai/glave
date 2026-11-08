//! `glave threads`: the threads in this folder, and archiving, unarchiving, deleting and
//! forking them, through the person's server (`reach.rs`), as the app does. A folder that is no
//! project has none, and is never made one.

use std::process::ExitCode;

use glave_event::{Body, Seq, ThreadId, decode};
use glave_harness::Change;
use glave_ports::Standing;
use glave_session::protocol::{Reply, Request, Show};
use uuid::Uuid;

use crate::client::read_events;
use crate::reach::{self, Reached};
use crate::turns::{Here, no_project};

pub(crate) const USAGE: &str = "\
glave threads: the threads in this folder

Usage:
  glave threads [--archived | --deleted]    list them, newest first: the active ones, or the
                                            archived, or the deleted (waiting for erasure)
  glave threads archive <id>                hide a thread from the list, keeping it
  glave threads unarchive <id>              bring an archived thread back
  glave threads delete <id>                 hide a thread for good and mark it for erasure
  glave threads fork <id> [--at <turn>]     a new thread with this one's history up to the end
                                            of its turn <turn> (0 for none of them), or of its
                                            latest; glave -c then goes on from there

Forking leaves the thread as it was, and its files as they are now: they are not rewound.
To send an earlier message again changed, fork at the turn before it and send it.

Archiving or deleting a thread removes its worktree when that loses nothing (no uncommitted
changes, every commit on a remote) and keeps it otherwise; glave worktrees says why. A
thread's content is erased by the operator of the deployment; today's local keys cannot erase
one thread alone (specs/storage.md), so a deleted thread stays in the database, hidden.";

/// The most threads listed.
const MOST_LISTED: usize = 1_000;

/// What was asked.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    List(Standing),
    Change(Change, ThreadId),
    /// Fork the thread after this many of its turns, or after its latest.
    Fork(ThreadId, Option<u64>),
}

fn parse(words: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let change = |change, id: &str| Ok(Command::Change(change, thread_id(id)?));
    let turns = |turn: &str| {
        turn.parse::<u64>()
            .map_err(|_| format!("--at takes a turn's number, not {turn:?}"))
    };
    match words.as_slice() {
        [] => Ok(Command::List(Standing::Active)),
        ["--archived"] => Ok(Command::List(Standing::Archived)),
        ["--deleted"] => Ok(Command::List(Standing::Deleted)),
        ["archive", id] => change(Change::Archive, id),
        ["unarchive", id] => change(Change::Unarchive, id),
        ["delete", id] => change(Change::Delete, id),
        ["fork", id] => Ok(Command::Fork(thread_id(id)?, None)),
        ["fork", id, "--at", turn] | ["fork", "--at", turn, id] => {
            Ok(Command::Fork(thread_id(id)?, Some(turns(turn)?)))
        }
        [first, ..] => Err(format!("unexpected {first:?}")),
    }
}

/// The thread `id` names.
fn thread_id(id: &str) -> Result<ThreadId, String> {
    let uuid = Uuid::parse_str(id).map_err(|e| format!("{id:?} is not a thread id: {e}"))?;
    ThreadId::try_from(uuid).map_err(|e| format!("{id:?} is not a thread id: {e}"))
}

pub(crate) fn main(arguments: &[String]) -> ExitCode {
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
    reach::with(async |server| through(server, command).await).await
}

/// `command` through the server reached.
async fn through(server: &mut Reached, command: Command) -> anyhow::Result<()> {
    match command {
        Command::List(standing) => {
            let show = match standing {
                Standing::Active => Show::Active,
                Standing::Archived => Show::Archived,
                Standing::Deleted => Show::Deleted,
            };
            let folder = std::env::current_dir()?;
            let project = match crate::turns::found(server.session(), &folder).await? {
                Here::Project(project) => Some(project),
                Here::Chats => None,
                Here::NoProject => return no_project(true),
            };
            let listed = Request::ThreadList {
                project,
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
        }
        Command::Change(change, thread) => {
            let request = match change {
                Change::Archive => Request::ThreadArchive { thread },
                Change::Unarchive => Request::ThreadUnarchive { thread },
                Change::Delete => Request::ThreadDelete { thread },
            };
            server.request(request).await?;
            println!("{} {thread}", done(change));
        }
        Command::Fork(thread, turns) => {
            let at = match turns {
                Some(turns) => after_turns(server, thread, turns).await?,
                None => None,
            };
            let Reply::Thread { thread: forked } =
                server.request(Request::ThreadFork { thread, at }).await?
            else {
                anyhow::bail!("glave answered thread/fork with something else");
            };
            println!("{}", forked.id);
            if let Some(told) = told(server, forked.id).await? {
                println!("{told}");
            }
        }
    }
    Ok(())
}

/// Where a fork after the first `turns` of `thread`'s turns ends: the end of the last of them,
/// or, for none, just before the first began; `None`, its latest point, for a thread with none.
async fn after_turns(
    server: &Reached,
    thread: ThreadId,
    turns: u64,
) -> anyhow::Result<Option<Seq>> {
    let (mut started, mut ended) = (None, Vec::new());
    read_events(server.session(), thread, |event| {
        match event.kind.as_str() {
            "turn.started" => started = started.or(Some(event.seq)),
            "turn.ended" => ended.push(event.seq),
            _ => {}
        }
        Ok(())
    })
    .await?;
    let Some(last) = turns.checked_sub(1) else {
        return Ok(started
            .map(|first| Seq::new(first.get().saturating_sub(1)))
            .transpose()?);
    };
    let end = usize::try_from(last).ok().and_then(|last| ended.get(last));
    match end {
        Some(end) => Ok(Some(*end)),
        None => anyhow::bail!("--at takes 0 to {} for thread {thread}", ended.len()),
    }
}

/// What the fork `thread` tells of its files, its `thread.forked`, the last of its events.
async fn told(server: &Reached, thread: ThreadId) -> anyhow::Result<Option<String>> {
    let page = Request::ThreadTurns {
        thread,
        before: None,
    };
    let Reply::Events { events, .. } = server.request(page).await? else {
        anyhow::bail!("glave answered thread/turns with something else");
    };
    let last = events.into_iter().last().map(decode).transpose()?;
    Ok(last.and_then(|event| match event.body {
        Body::ThreadForked(forked) => Some(forked.told),
        _ => None,
    }))
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
        assert!(matches!(
            parse(&words(&["fork", id])),
            Ok(Command::Fork(_, None))
        ));
        for asked in [["fork", id, "--at", "2"], ["fork", "--at", "2", id]] {
            assert!(matches!(
                parse(&words(&asked)),
                Ok(Command::Fork(_, Some(2)))
            ));
        }
        assert!(parse(&words(&["fork", id, "--at", "last"])).is_err());
        assert!(parse(&words(&["fork"])).is_err());
    }
}
