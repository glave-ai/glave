//! A person's computer that loses its machine (`specs/computers.md`, decision 16): the call it
//! cut off says what stopped it and why, the next call starts a new machine on the same disk,
//! and what ran in the background is told as stopped, never started again, in the first tool
//! result since or the note before the person's next message: always at the end of the
//! conversation, never by changing what every call opens with.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use glave_computer::{Grow, LocalComputers, Lost, Peek, Wake, Waking};
use glave_event::{Body, CommandEnding, PrincipalId, StopReason};
use glave_harness::{AllowAll, Homes, Place};
use glave_ports::computer::{Command, ComputerError, Home, MOVED, Sandbox};
use glave_ports::testing::{Captured, TempDir};
use glave_ports::{InMemoryLog, ModelRequest, NeverCancelled, OrgId};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::support::*;
use crate::wire::home_across_the_wire;

/// The named pipe a command opens as it starts: read once it is running, so the test takes the
/// machine away from under it.
const STARTED: &str = "started";

/// A machine a person's computer runs on: this one's computers across the wire, all of it gone
/// when this is, what ran there with it, as a Pod's programs go with the Pod.
struct Machine {
    home: Home,
    serving: JoinHandle<io::Result<()>>,
}

impl Drop for Machine {
    fn drop(&mut self) {
        self.serving.abort();
    }
}

/// Starts a machine with the disk at `folder`.
async fn started(base: &Path, folder: &Path) -> Machine {
    let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
    let computers = Arc::new(LocalComputers::new(base.join("copies"), environment));
    let (home, serving) = home_across_the_wire(computers, folder).await;
    Machine { home, serving }
}

/// A person's computer that loses its machine when the test says so, its disk `folder`; each
/// machine one of its own, as each Pod is, behind a [`Waking`] as a server holds it.
struct Moving {
    planned: Home,
    /// The machine it runs on, while it has one.
    machine: Arc<Mutex<Option<Machine>>>,
    lost: Arc<AtomicBool>,
    woken: Arc<AtomicUsize>,
}

impl Moving {
    /// The computer `folder` is the disk of, on a machine of its own.
    async fn new(base: &Path, folder: &Path) -> Arc<Self> {
        let first = started(base, folder).await;
        let planned_home = first.home.home.clone();
        let machine = Arc::new(Mutex::new(Some(first)));
        let lost = Arc::new(AtomicBool::new(false));
        let woken = Arc::new(AtomicUsize::new(0));
        let (runs, gone, count) = (Arc::clone(&machine), Arc::clone(&lost), Arc::clone(&woken));
        let (base_at, at) = (base.to_owned(), folder.to_owned());
        let wake: Wake = Arc::new(move || {
            let (runs, gone, count) = (Arc::clone(&runs), Arc::clone(&gone), Arc::clone(&count));
            let (base, at) = (base_at.clone(), at.clone());
            Box::pin(async move {
                let mut machine = runs.lock().await;
                if let Some(running) = machine.as_ref() {
                    return Ok(running.home.clone());
                }
                count.fetch_add(1, Ordering::SeqCst);
                let new = machine.insert(started(&base, &at).await);
                gone.store(false, Ordering::SeqCst);
                Ok(new.home.clone())
            })
        });
        let seen = Arc::clone(&machine);
        let peek: Peek = Arc::new(move || {
            let seen = Arc::clone(&seen);
            Box::pin(async move { Ok(seen.lock().await.as_ref().map(|m| m.home.clone())) })
        });
        let grow: Grow = Arc::new(|_| Box::pin(std::future::ready(None)));
        let looks = Arc::clone(&lost);
        let is_lost: Lost = Arc::new(move || {
            let lost = looks.load(Ordering::SeqCst);
            Box::pin(std::future::ready(lost))
        });
        let scratch = folder.join("scratch").to_string_lossy().into_owned();
        let planned = Waking::home((wake, peek), Some((grow, is_lost)), planned_home, &scratch);
        Arc::new(Self {
            planned,
            machine,
            lost,
            woken,
        })
    }

    /// Takes its machine away, and what ran on it.
    async fn lose(&self) {
        self.lost.store(true, Ordering::SeqCst);
        self.machine.lock().await.take();
    }

    /// Drops the connection to its machine, which stays.
    async fn drop_connection(&self) {
        if let Some(machine) = self.machine.lock().await.as_ref() {
            machine.serving.abort();
        }
    }

    fn woken(&self) -> usize {
        self.woken.load(Ordering::SeqCst)
    }

    /// Runs `script` on its machine, as the test sets things up.
    async fn run(&self, script: &str) {
        let Some(home) = self.machine.lock().await.as_ref().map(|m| m.home.clone()) else {
            panic!("no machine to run {script} on");
        };
        home.computers
            .run(
                home.home.id,
                &Command::new(script),
                &Captured::default(),
                &NeverCancelled,
            )
            .await
            .unwrap();
    }
}

impl Homes for Moving {
    fn home(&self, _org: OrgId, _person: PrincipalId) -> Result<Home, ComputerError> {
        Ok(self.planned.clone())
    }

    fn hold(&self, _org: OrgId, _person: PrincipalId) -> Result<Box<dyn Send>, ComputerError> {
        Ok(Box::new(()))
    }

    fn sandbox(&self) -> Option<Sandbox> {
        None
    }
}

/// An answer calling `tool` with `arguments`.
fn calls(tool: &str, arguments: Value) -> glave_ports::Completion {
    answer(None, vec![(tool, arguments)], StopReason::ToolUse)
}

fn bash(script: &str) -> glave_ports::Completion {
    calls("bash", json!({ "command": script }))
}

fn background(script: &str) -> glave_ports::Completion {
    calls("bash", json!({ "command": script, "background": true }))
}

fn done() -> glave_ports::Completion {
    answer(Some("Done."), vec![], StopReason::EndTurn)
}

/// A person's folder on a computer that loses its machine: its folder, and the place a thread
/// opens it at.
fn place_at(computer: &TempDir) -> (PathBuf, Place) {
    let folder = computer.join("home");
    std::fs::create_dir_all(folder.join(".git")).unwrap();
    let path = folder.to_string_lossy().into_owned();
    (folder, Place::Folder(path))
}

/// The last message of the request at `index`, as the model was sent it.
fn last_message(requests: &[ModelRequest], index: usize) -> String {
    let messages = &requests[index].messages;
    texts(&messages[messages.len() - 1]).join("\n")
}

/// What every call of `requests` opens with never changed: the system prompt and its tools.
fn prefix_is_unchanged(requests: &[ModelRequest]) {
    let (first, rest) = requests.split_first().unwrap();
    for request in rest {
        assert_eq!(request.system, first.system);
        assert_eq!(
            serde_json::to_vec(&request.tools).unwrap(),
            serde_json::to_vec(&first.tools).unwrap()
        );
    }
}

/// Waits for the command started in `folder` to open its pipe, then does `then` under it.
async fn once_started(folder: &Path, then: impl Future<Output = ()>) {
    let pipe = folder.join(STARTED);
    tokio::task::spawn_blocking(move || std::fs::read(pipe).unwrap())
        .await
        .unwrap();
    then.await;
}

#[test]
fn a_command_cut_off_with_its_machine_says_why_and_the_next_call_runs_on_the_same_disk() {
    let (server, computer) = (folder("moved-server"), folder("moved-computer"));
    let (home, place) = place_at(&computer);
    block_on!(async {
        let moving = Moving::new(&computer, &home).await;
        moving.run(&format!("mkfifo {STARTED}")).await;
        let answers = vec![
            bash(&format!(
                "echo before > kept.txt; echo ran >> runs.txt; echo > {STARTED}; sleep 30"
            )),
            bash("cat kept.txt; cat runs.txt"),
            done(),
        ];
        let (harness, seen) = harness_seeing(
            &server,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let harness = harness.with_homes(Some(Arc::clone(&moving) as Arc<dyn Homes>));
        let mut thread = thread_in(&harness, place).await;
        tokio::join!(
            turn(&harness, &mut thread, "build it"),
            once_started(&home, moving.lose())
        );

        let requests = seen.lock().unwrap().clone();
        let cut_off = last_message(&requests, 1);
        assert!(cut_off.contains(MOVED), "{cut_off}");
        assert!(cut_off.contains("this command was stopped"), "{cut_off}");
        assert!(
            cut_off.contains("Run the command again if it is still needed"),
            "the model runs it again if it still wants it: {cut_off}"
        );
        let next = last_message(&requests, 2);
        assert!(
            next.contains("before") && next.contains("ran"),
            "on the same disk: {next}"
        );
        assert_eq!(moving.woken(), 1, "the next call started a new machine");
        let runs = std::fs::read_to_string(home.join("runs.txt")).unwrap();
        assert_eq!(runs, "ran\n", "glave never ran it again");
        prefix_is_unchanged(&requests);
    });
}

#[test]
fn what_ran_in_the_background_is_told_as_stopped_once_in_the_note_before_the_next_message() {
    let (server, computer) = (folder("moved-note-server"), folder("moved-note-computer"));
    let (home, place) = place_at(&computer);
    block_on!(async {
        let moving = Moving::new(&computer, &home).await;
        let answers = vec![
            background("sleep 30"),
            done(),
            calls("command_output", json!({ "command": 1 })),
            done(),
            done(),
        ];
        let log = InMemoryLog::new();
        let (harness, seen) = harness_seeing(&server, answers, AllowAll, log, 1, settings()).await;
        let harness = harness.with_homes(Some(Arc::clone(&moving) as Arc<dyn Homes>));
        let mut thread = thread_in(&harness, place).await;
        turn(&harness, &mut thread, "start the server").await;

        // The machine goes while no turn runs, and the wait a server keeps on the command ends.
        moving.lose().await;
        thread.commands().until_one_ends().await;
        harness.record_commands(&mut thread).await.unwrap();
        let events = events(harness.log(), thread.id).await;
        let ended = events.iter().find_map(|event| match &event.body {
            Body::CommandEnded(ended) => Some(ended.ending.clone()),
            _ => None,
        });
        assert_eq!(ended, Some(CommandEnding::Gone), "recorded at once");

        // The next message is told, in the note before the person's words, once.
        turn(&harness, &mut thread, "is it up?").await;
        turn(&harness, &mut thread, "and now?").await;
        let requests = seen.lock().unwrap().clone();
        let told = last_message(&requests, 2);
        assert!(told.starts_with("<system-reminder>\n"), "{told}");
        assert!(told.contains("is it up?"), "{told}");
        assert!(told.contains(&format!("glave: {MOVED}")), "{told}");
        assert!(told.contains("command 1 (sleep 30)"), "{told}");
        assert!(
            told.contains("does not start them again"),
            "never started again by glave: {told}"
        );

        // Reading it says the same.
        let read = last_message(&requests, 3);
        assert!(
            read.contains(&format!(
                "Command 1 was stopped when {MOVED}: there is no more of its output."
            )),
            "{read}"
        );
        let later = last_message(&requests, 4);
        assert!(!later.contains(MOVED), "told once: {later}");
        assert_eq!(moving.woken(), 0, "only a call that needs it starts one");
        prefix_is_unchanged(&requests);
    });
}

#[test]
fn the_first_tool_result_after_the_machine_went_names_what_ran_in_the_background() {
    let (server, computer) = (
        folder("moved-result-server"),
        folder("moved-result-computer"),
    );
    let (home, place) = place_at(&computer);
    block_on!(async {
        let moving = Moving::new(&computer, &home).await;
        moving.run(&format!("mkfifo {STARTED}")).await;
        let answers = vec![
            background("sleep 31\necho a second line"),
            bash(&format!("echo > {STARTED}; sleep 30")),
            bash("echo again"),
            done(),
            done(),
        ];
        let (harness, seen) = harness_seeing(
            &server,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let harness = harness.with_homes(Some(Arc::clone(&moving) as Arc<dyn Homes>));
        let mut thread = thread_in(&harness, place).await;
        tokio::join!(
            turn(&harness, &mut thread, "start and build"),
            once_started(&home, moving.lose())
        );
        let requests = seen.lock().unwrap().clone();
        let result = last_message(&requests, 2);
        assert!(result.contains("this command was stopped"), "{result}");
        assert!(
            result.contains("[glave:") && result.contains("command 1 (sleep 31)"),
            "the same result names what ran in the background: {result}"
        );
        assert!(!result.contains("a second line"), "one line each: {result}");
        let next = last_message(&requests, 3);
        assert!(!next.contains("command 1"), "told once: {next}");
        prefix_is_unchanged(&requests);
    });
}

#[test]
fn a_server_s_rollout_tells_a_thread_read_from_its_log_what_stopped() {
    let (server, computer) = (
        folder("moved-rollout-server"),
        folder("moved-rollout-computer"),
    );
    let (home, place) = place_at(&computer);
    block_on!(async {
        let first = Moving::new(&computer, &home).await;
        let answers = vec![background("sleep 30"), done(), done(), done()];
        let (harness, seen) = harness_seeing(
            &server,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let harness = harness.with_homes(Some(Arc::clone(&first) as Arc<dyn Homes>));
        let mut thread = thread_in(&harness, place).await;
        turn(&harness, &mut thread, "start the server").await;

        // A new server, which never started this computer, replaces it at its first call.
        first.lose().await;
        let next = Moving::new(&computer, &home).await;
        let harness = harness.with_homes(Some(Arc::clone(&next) as Arc<dyn Homes>));
        let mut resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut resumed, "is it up?").await;
        let requests = seen.lock().unwrap().clone();
        let told = last_message(&requests, 2);
        assert!(told.contains("command 1 (sleep 30)"), "{told}");
        assert!(told.contains(MOVED), "{told}");

        // Told at the turn its log recorded, so a server that reads the thread again doesn't
        // tell it twice.
        let mut again = harness.resume_thread(scope(), thread.id).await.unwrap();
        turn(&harness, &mut again, "and now?").await;
        let requests = seen.lock().unwrap().clone();
        let later = last_message(&requests, 3);
        assert!(!later.contains(MOVED), "told once: {later}");
        prefix_is_unchanged(&requests);
    });
}

/// Only a person's own computer can move: on this one a command no longer there has just ended.
#[test]
fn a_command_gone_from_a_computer_that_cannot_move_is_told_as_ever() {
    let dir = folder("moved-local");
    block_on!(async {
        let answers = vec![
            background("sleep 30"),
            done(),
            calls("command_output", json!({ "command": 1, "stop": true })),
            calls("command_output", json!({ "command": 1 })),
            done(),
        ];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        turn(&harness, &mut thread, "start the server").await;
        turn(&harness, &mut thread, "now stop it").await;
        let requests = seen.lock().unwrap().clone();
        let everything: Vec<String> = requests
            .iter()
            .flat_map(|request| &request.messages)
            .flat_map(texts)
            .collect();
        assert!(
            !everything.iter().any(|text| text.contains(MOVED)),
            "{everything:?}"
        );
    });
}

/// Only a lost machine is told as one: a connection that dropped, the machine still there, is
/// told as ever, and nothing is said to have moved.
#[test]
fn a_connection_that_dropped_with_its_machine_still_there_is_not_told_as_a_move() {
    let (server, computer) = (folder("moved-kept-server"), folder("moved-kept-computer"));
    let (home, place) = place_at(&computer);
    block_on!(async {
        let moving = Moving::new(&computer, &home).await;
        moving.run(&format!("mkfifo {STARTED}")).await;
        let answers = vec![
            bash(&format!("echo > {STARTED}; sleep 30")),
            answer(Some("It dropped."), vec![], StopReason::EndTurn),
        ];
        let (harness, seen) = harness_seeing(
            &server,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            settings(),
        )
        .await;
        let harness = harness.with_homes(Some(Arc::clone(&moving) as Arc<dyn Homes>));
        let mut thread = thread_in(&harness, place).await;
        tokio::join!(
            turn(&harness, &mut thread, "build it"),
            once_started(&home, moving.drop_connection())
        );
        let requests = seen.lock().unwrap().clone();
        let said = last_message(&requests, 1);
        assert!(said.contains("may or may not have done it"), "{said}");
        assert!(!said.contains(MOVED), "{said}");
    });
}
