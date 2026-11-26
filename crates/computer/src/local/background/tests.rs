use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_ports::block_on;
use glave_ports::computer::{
    Command, CommandId, ComputerId, Computers, Ending, Finished, RunsFor, WorkspaceId,
};
use glave_ports::testing::TempDir;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use super::{
    Backgrounds, Ended, Entry, MAX_KEPT, MAX_RUNNING, OTHERS_KEPT, OTHERS_RUNNING, Shared, full,
    prune,
};
use crate::LocalComputers;
use crate::local::new_id;
use crate::testing::ran;

fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("background-{name}")).unwrap()
}

fn environment() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect()
}

/// Polls in the shell, so no test sleeps: true once the process in `file` is gone.
const GONE: &str = "for _ in $(seq 100); do kill -0 \"$(cat child.pid)\" 2>/dev/null || exit 0; \
     sleep 0.05; done; exit 1";

#[test]
fn a_command_nothing_reads_is_stopped_once_idle() {
    let dir = folder("idle");
    block_on!(async {
        let mut computers = LocalComputers::for_here(environment());
        computers.background = Backgrounds::new(Duration::from_secs(1));
        let (computers, here) = computers.open_here(&dir).await.unwrap();
        let script = "sleep 300 & echo $! > child.pid; wait";
        let id = computers
            .start(here.id, &Command::new(script))
            .await
            .unwrap();
        // Listing is not reading: listed all along, it still goes idle. Each wait is on its
        // ending, not a margin, so a loaded machine only makes the loop go round fewer times.
        let tick = Duration::from_millis(200);
        let mut finished = None;
        for _ in 0..100 {
            let listed = computers.commands(here.id).await.unwrap();
            assert!(listed.iter().any(|listed| listed.id == id));
            finished = computers.ended(here.id, id, tick).await.unwrap();
            if finished.is_some() {
                break;
            }
        }
        let finished = finished.expect("it went idle while it was only listed");
        assert_eq!(finished.ending, Ending::Idle { after_secs: 1 });
        assert!(finished.millis >= 1000, "{finished:?}");
        // Its child is stopped too.
        ran(computers.as_ref(), here.id, GONE).await;
    });
}

#[test]
fn every_command_still_running_is_stopped_when_the_computers_go() {
    let dir = folder("dropped");
    block_on!(async {
        let (computers, here) = LocalComputers::here(&dir, environment()).await.unwrap();
        let script = "sleep 300 & echo $! > child.pid; echo ready; wait";
        let id = computers
            .start(here.id, &Command::new(script))
            .await
            .unwrap();
        let wait = Duration::from_millis(100);
        let mut seen = String::new();
        while !seen.contains("ready") {
            seen.push_str(&computers.output(here.id, id, wait).await.unwrap().text);
        }
        drop(computers);
        let (computers, here) = LocalComputers::here(&dir, environment()).await.unwrap();
        // Nothing it started is left.
        ran(computers.as_ref(), here.id, GONE).await;
    });
}

#[test]
fn a_command_that_ends_takes_what_it_left_running_with_it() {
    let dir = folder("left");
    block_on!(async {
        let (computers, here) = LocalComputers::here(&dir, environment()).await.unwrap();
        let script = "sleep 300 > /dev/null & echo $! > child.pid; echo started";
        let id = computers
            .start(here.id, &Command::new(script))
            .await
            .unwrap();
        let wait = Duration::from_secs(20);
        let printed = computers.output(here.id, id, wait).await.unwrap();
        assert_eq!(
            printed.ended.map(|ended| ended.ending),
            Some(Ending::Exited { code: 0 }),
            "{printed:?}"
        );
        ran(computers.as_ref(), here.id, GONE).await;
    });
}

/// One command held for `runs_for` in a workspace of its own, ended `ended` seconds after
/// `base` when it has, its watcher a task with nothing to do.
fn held(runs_for: RunsFor, base: Instant, ended: Option<u64>) -> (CommandId, Entry) {
    let ended = ended.map(|secs| Ended {
        finished: Finished::new(Ending::Exited { code: 0 }, 0),
        at: base + Duration::from_secs(secs),
    });
    let shared = Arc::new(Shared {
        unread: Mutex::default(),
        read_at: Mutex::new(base),
        ended: watch::channel(ended).0,
        stop: Notify::new(),
    });
    let entry = Entry {
        computer: ComputerId::from_uuid(new_id().unwrap()),
        workspace: WorkspaceId::from_uuid(new_id().unwrap()),
        runs_for,
        script: String::new(),
        started: base,
        group: None,
        shared,
        watcher: tokio::spawn(async {}),
    };
    (CommandId::from_uuid(new_id().unwrap()), entry)
}

#[test]
fn everyone_else_s_commands_leave_the_owner_room_to_run() {
    block_on!(async {
        let base = Instant::now();
        let mut commands: HashMap<_, _> = (0..OTHERS_RUNNING)
            .map(|_| held(RunsFor::SomeoneElse, base, None))
            .collect();
        let anywhere = WorkspaceId::from_uuid(new_id().unwrap());
        assert!(
            full(&commands, anywhere, RunsFor::SomeoneElse).is_some(),
            "everyone else's share is full"
        );
        assert!(
            full(&commands, anywhere, RunsFor::Owner).is_none(),
            "the owner still has room"
        );
        commands.extend((OTHERS_RUNNING..MAX_RUNNING).map(|_| held(RunsFor::Owner, base, None)));
        assert!(
            full(&commands, anywhere, RunsFor::Owner).is_some(),
            "the machine is full"
        );
    });
}

#[test]
fn everyone_else_s_ended_commands_are_forgotten_before_the_owner_s() {
    block_on!(async {
        let base = Instant::now();
        // The owner's ended first, so the oldest of all are theirs.
        let mut commands: HashMap<_, _> = (0..40)
            .map(|at| held(RunsFor::Owner, base, Some(at)))
            .chain((0..90).map(|at| held(RunsFor::SomeoneElse, base, Some(1000 + at))))
            .collect();
        prune(&mut commands);
        let owner_s = commands
            .values()
            .filter(|entry| entry.runs_for == RunsFor::Owner)
            .count();
        assert_eq!(owner_s, 40, "none of the owner's is forgotten");
        assert_eq!(commands.len(), MAX_KEPT);

        let mut others: HashMap<_, _> = (0..100)
            .map(|at| held(RunsFor::SomeoneElse, base, Some(at)))
            .collect();
        prune(&mut others);
        assert_eq!(
            others.len(),
            OTHERS_KEPT,
            "everyone else keeps their share alone"
        );
    });
}
