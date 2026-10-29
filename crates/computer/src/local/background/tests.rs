use std::sync::Mutex;

use glave_ports::computer::{Command, Computers, Ending, Output, WorkspaceInfo};
use glave_ports::testing::TempDir;
use glave_ports::{BoxFuture, NeverCancelled, block_on};

use super::Backgrounds;
use crate::LocalComputers;

fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("background-{name}")).unwrap()
}

fn environment() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect()
}

#[derive(Default)]
struct Collect(Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push_str(text);
        Box::pin(std::future::ready(()))
    }
}

/// Runs `script` in the foreground and says whether it exited 0.
async fn succeeds(computers: &LocalComputers, here: &WorkspaceInfo, script: &str) -> bool {
    let finished = computers
        .run(
            here.id,
            &Command::new(script),
            &Collect::default(),
            &NeverCancelled,
        )
        .await
        .unwrap();
    finished.ending == Ending::Exited { code: 0 }
}

/// Polls in the shell, so no test sleeps: true once the process in `file` is gone.
const GONE: &str = "for _ in $(seq 100); do kill -0 \"$(cat child.pid)\" 2>/dev/null || exit 0; \
     sleep 0.05; done; exit 1";

#[test]
fn a_command_nothing_reads_is_stopped_once_idle() {
    let dir = folder("idle");
    block_on!(async {
        let mut computers = LocalComputers::for_here(environment());
        computers.background = Backgrounds::new(std::time::Duration::from_secs(1));
        let (computers, here) = computers.open_here(&dir).await.unwrap();
        let script = "sleep 300 & echo $! > child.pid; wait";
        let id = computers
            .start(here.id, &Command::new(script))
            .await
            .unwrap();
        // Listing is not reading: listed all along, it still goes idle. Each wait is on its
        // ending, not a margin, so a loaded machine only makes the loop go round fewer times.
        let tick = std::time::Duration::from_millis(200);
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
        assert!(
            succeeds(&computers, &here, GONE).await,
            "its child is stopped too"
        );
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
        let wait = std::time::Duration::from_millis(100);
        let mut seen = String::new();
        while !seen.contains("ready") {
            seen.push_str(&computers.output(here.id, id, wait).await.unwrap().text);
        }
        drop(computers);
        let (computers, here) = LocalComputers::here(&dir, environment()).await.unwrap();
        assert!(
            succeeds(&computers, &here, GONE).await,
            "nothing it started is left"
        );
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
        let wait = std::time::Duration::from_secs(20);
        let printed = computers.output(here.id, id, wait).await.unwrap();
        assert_eq!(
            printed.ended.map(|ended| ended.ending),
            Some(Ending::Exited { code: 0 }),
            "{printed:?}"
        );
        assert!(succeeds(&computers, &here, GONE).await);
    });
}
