use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use rustix::process::{Pid, test_kill_process};

use super::*;
use crate::ledger::{Process, Record};
use crate::start::{self, Start};
use crate::testing::{Orphan, entries};

fn folder(name: &str) -> glave_ports::testing::TempDir {
    glave_ports::testing::TempDir::new(&format!("process-{name}")).unwrap()
}

/// A glave that has ended: a process that ran and was reaped, with a start no process has.
fn gone_glave() -> Process {
    let mut ran = std::process::Command::new("true").spawn().unwrap();
    let pid = i32::try_from(ran.id()).unwrap();
    ran.wait().unwrap();
    Process {
        pid,
        start: Start("ended".into()),
    }
}

/// Writes an entry as a glave would have.
fn plant(folder: &Path, process: Process, glave: Process) {
    std::fs::create_dir_all(folder).unwrap();
    let record = Record {
        version: 1,
        group: process.pid,
        process,
        owner: Owner::Workspace("w".into()),
        glave,
    };
    let path = folder.join(format!("{}.json", record.process.pid));
    std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

#[test]
fn a_process_is_recorded_while_it_runs_and_its_group_goes_with_it() {
    let run = folder("recorded");
    let processes = Processes::recorded(&run).owned_by(Owner::Workspace("w".into()));
    let ledger = run.join("processes");
    let mut program = Program::new("sh");
    program
        .args(["-c", "sleep 300 & echo $!; wait"])
        .no_input()
        .stdout(Stdio::piped());
    let (child, left) = glave_ports::block_on!(async {
        let mut child = processes.spawn(&mut program).await.unwrap();
        let stdout = child.stdout.take().unwrap().into_owned_fd().unwrap();
        let mut line = String::new();
        BufReader::new(std::fs::File::from(stdout))
            .read_line(&mut line)
            .unwrap();
        (child, line.trim().parse::<i32>().unwrap())
    });
    assert_eq!(entries(&ledger), 1);
    let pid = i32::try_from(child.id().unwrap()).unwrap();
    let text = std::fs::read(ledger.join(format!("{pid}.json"))).unwrap();
    let record: Record = serde_json::from_slice(&text).unwrap();
    assert_eq!((record.group, record.process.pid), (pid, pid));
    assert_eq!(record.owner, Owner::Workspace("w".into()));
    assert_eq!(record.glave.pid, i32::try_from(std::process::id()).unwrap());
    drop(child);
    assert_eq!(entries(&ledger), 0, "an entry goes with its process");
    let deadline = Instant::now() + Duration::from_secs(10);
    while test_kill_process(Pid::from_raw(left).unwrap()).is_ok() {
        assert!(
            Instant::now() < deadline,
            "what it started must stop with it"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = std::fs::remove_dir_all(run);
}

#[test]
fn what_a_glave_that_is_gone_left_is_stopped_at_the_next_start() {
    let run = folder("reaped");
    let ledger = run.join("processes");
    let left = Orphan::start();
    let process = Process {
        pid: left.pid,
        start: left.start_time(),
    };
    plant(&ledger, process, gone_glave());
    let stopped = glave_ports::block_on!(Processes::recorded(&run).reap()).unwrap();
    assert_eq!(stopped, 1);
    assert!(left.ended(), "a gone glave's process must be stopped");
    assert_eq!(entries(&ledger), 0);
    let _ = std::fs::remove_dir_all(run);
}

#[test]
fn an_entry_that_cannot_be_read_is_skipped_and_the_others_are_still_stopped() {
    let run = folder("garbage");
    let ledger = run.join("processes");
    let left = Orphan::start();
    let process = Process {
        pid: left.pid,
        start: left.start_time(),
    };
    plant(&ledger, process, gone_glave());
    std::fs::write(ledger.join("1.json"), b"{\"version\": 1, \"proc").unwrap();
    std::fs::write(ledger.join("2.json"), b"\x00\xff not an entry").unwrap();
    let stopped = glave_ports::block_on!(Processes::recorded(&run).reap()).unwrap();
    assert_eq!(stopped, 1);
    assert!(
        left.ended(),
        "the process recorded beside them is still found by its pid and stopped"
    );
    assert_eq!(entries(&ledger), 2, "what cannot be read is left as it is");
    let _ = std::fs::remove_dir_all(run);
}

#[test]
fn a_reused_pid_is_never_killed() {
    let run = folder("reused");
    let ledger = run.join("processes");
    let other = Orphan::start();
    // The entry names the process's id, but another start: the process recorded ended, and
    // this one was given its number since.
    let recorded = Process {
        pid: other.pid,
        start: Start("an earlier process".into()),
    };
    plant(&ledger, recorded, gone_glave());
    let stopped = glave_ports::block_on!(Processes::recorded(&run).reap()).unwrap();
    assert_eq!(stopped, 0);
    assert!(
        other.alive(),
        "a process with a recorded id but another start is not glave's"
    );
    assert_eq!(entries(&ledger), 0, "the stale entry is forgotten");
    let _ = std::fs::remove_dir_all(run);
}

#[test]
fn a_running_glave_s_processes_are_left_alone() {
    let run = folder("live");
    let ledger = run.join("processes");
    let mine = Orphan::start();
    let me = i32::try_from(std::process::id()).unwrap();
    let glave = Process {
        pid: me,
        start: start::of(&[me]).remove(&me).unwrap(),
    };
    let process = Process {
        pid: mine.pid,
        start: mine.start_time(),
    };
    plant(&ledger, process, glave);
    let stopped = glave_ports::block_on!(Processes::recorded(&run).reap()).unwrap();
    assert_eq!(stopped, 0);
    assert!(mine.alive());
    assert_eq!(entries(&ledger), 1);
    let _ = std::fs::remove_dir_all(run);
}

#[test]
fn the_ledger_is_bounded_and_forgets_what_ended_to_make_room() {
    let run = folder("bounded");
    let ledger = run.join("processes");
    let processes = Processes::with_ledger(Ledger::new(ledger.clone(), 2));
    for _ in 0..2 {
        let ended = gone_glave();
        plant(&ledger, ended, gone_glave());
    }
    let sleep = || {
        let mut program = Program::new("sleep");
        program.arg("300");
        program
    };
    glave_ports::block_on!(async {
        let first = processes.spawn(&mut sleep()).await.unwrap();
        assert_eq!(entries(&ledger), 1, "entries whose process ended make room");
        let second = processes.spawn(&mut sleep()).await.unwrap();
        let refused = processes.spawn(&mut sleep()).await.unwrap_err();
        assert!(matches!(refused, ProcessError::Full(2)), "{refused}");
        assert_eq!(entries(&ledger), 2);
        drop(first);
        let third = processes.spawn(&mut sleep()).await.unwrap();
        drop((second, third));
    });
    assert_eq!(entries(&ledger), 0);
    let _ = std::fs::remove_dir_all(run);
}

/// Joins the process group its one argument names, says `in` (or why it could not), and
/// sleeps; its parent stops itself, so once killed it waits to be reaped, and nothing does.
#[cfg(target_os = "linux")]
const STUCK_IN_GROUP: &str = r#"
my $group = shift;
if (fork() == 0) {
    $| = 1;
    if (setpgrp(0, $group)) { print "in\n"; exec("sleep", "300"); }
    print "out: $!\n";
    exit 1;
}
kill("STOP", $$);
"#;

// Linux keeps a process that ended in its group until it is reaped; macOS takes it out at
// once, so there nothing a test can make stands for a process SIGKILL cannot end.
#[cfg(target_os = "linux")]
#[test]
fn what_sigkill_cannot_end_is_left_behind_recorded_and_glave_moves_on() {
    use std::os::unix::process::CommandExt as _;

    let run = folder("left-behind");
    let processes = Processes::recorded(&run).owned_by(Owner::Workspace("w".into()));
    let ledger = run.join("processes");
    let mut program = Program::new("sleep");
    program.arg("300").no_input();
    let (mut child, mut stuck) = glave_ports::block_on!(async {
        let child = processes.spawn(&mut program).await.unwrap();
        // A process in the child's group that SIGKILL ends but nothing reaps keeps the group
        // from emptying, as a process stuck in the kernel (a hung network mount) would.
        let group = child.id().unwrap().to_string();
        let stuck = std::process::Command::new("perl")
            .args(["-e", STUCK_IN_GROUP, &group])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        (child, stuck)
    });
    let mut said = String::new();
    BufReader::new(stuck.stdout.take().unwrap())
        .read_line(&mut said)
        .unwrap();
    assert_eq!(said, "in\n");

    let started = Instant::now();
    glave_ports::block_on!(child.stop());
    let took = started.elapsed();
    assert!(
        took < crate::group::GRACE * 3,
        "a group that does not empty holds the stop for two graces at most"
    );
    drop(child);
    assert_eq!(entries(&ledger), 1, "what was left behind stays recorded");

    let parent = Pid::from_raw(i32::try_from(stuck.id()).unwrap()).unwrap();
    let _ = rustix::process::kill_process_group(parent, rustix::process::Signal::KILL);
    let _ = stuck.wait();
    let _ = std::fs::remove_dir_all(run);
}
