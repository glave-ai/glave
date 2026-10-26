//! `glave threads`: the threads in this folder, and archiving, unarchiving and deleting them,
//! through the person's server (`reach.rs`), as the app does. A folder that is no project has
//! none, and is never made one.

use std::process::ExitCode;

use glave_event::ThreadId;
use glave_harness::Change;
use glave_ports::Standing;
use glave_session::protocol::{Reply, Request, Show};
use uuid::Uuid;

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
