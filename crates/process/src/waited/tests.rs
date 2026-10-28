use std::time::Instant;

use rustix::process::{Pid, Signal, kill_process, test_kill_process};

use super::*;
use crate::owner::Owner;
use crate::testing::entries;

const AMPLE: Duration = Duration::from_secs(30);

/// Waits until `pid` is gone, or fails, killing it, once the guard against a hang runs out.
fn gone(pid: i32, what: &str) {
    let process = Pid::from_raw(pid).unwrap();
    let deadline = Instant::now() + glave_ports::testing::HUNG;
    while test_kill_process(process).is_ok() {
        if Instant::now() >= deadline {
            let _ = kill_process(process, Signal::KILL);
            panic!("{what}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_program_that_ends_in_time_gives_what_it_printed() {
    let processes = Processes::unrecorded();
    let (printed, echoed) = glave_ports::block_on!(async {
        let printed = processes
            .output(Program::new("printf").arg("done"), AMPLE)
            .await;
        let echoed = processes
            .output_with(Program::new("cat").stdout(Stdio::piped()), b"given", AMPLE)
            .await;
        (printed, echoed)
    });
    assert_eq!(printed.unwrap().stdout, b"done");
    assert_eq!(echoed.unwrap().stdout, b"given");
}

#[test]
fn one_still_running_at_its_limit_is_stopped_saying_how_long_it_had() {
    let limit = Duration::from_secs(10);
    let processes = Processes::unrecorded();
    let (waited, fed) = glave_ports::block_on!(async {
        // The clock jumps to the limit as soon as nothing else can run.
        tokio::time::pause();
        let waited = processes
            .output(Program::new("sleep").arg("300"), limit)
            .await;
        let fed = processes
            .output_with(Program::new("sleep").arg("300"), b"unread", limit)
            .await;
        (waited, fed)
    });
    for error in [waited.unwrap_err(), fed.unwrap_err()] {
        assert!(matches!(error, ProcessError::TimedOut { .. }), "{error:?}");
        assert_eq!(error.to_string(), "sleep did not finish within 10 seconds");
    }
}

#[test]
fn at_its_limit_what_it_started_is_stopped_with_it() {
    let folder = glave_ports::testing::TempDir::new("waited-limit").unwrap();
    let said = folder.join("pid");
    // The shell starts a sleep of its own, says which, and waits for it.
    let script = format!("sleep 300 & echo $! > '{}'; wait", said.display());
    let processes = Processes::unrecorded();
    let ran = glave_ports::block_on!(async {
        tokio::time::pause();
        let mut program = Program::new("sh");
        program.args(["-c", &script]);
        // Busy until the shell has said which, so the clock jumps to the limit only then.
        let file = said.clone();
        let saying = tokio::spawn(async move {
            while !std::fs::read_to_string(&file).is_ok_and(|pid| pid.ends_with('\n')) {
                tokio::task::yield_now().await;
            }
        });
        let ran = processes
            .output(&mut program, Duration::from_secs(10))
            .await;
        saying.await.unwrap();
        ran
    });
    assert!(matches!(ran, Err(ProcessError::TimedOut { .. })), "{ran:?}");
    let pid = std::fs::read_to_string(&said)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    gone(pid, "the sleep it started outlived it");
}

#[test]
fn what_it_starts_stops_when_its_caller_stops_waiting() {
    let folder = glave_ports::testing::TempDir::new("waited-dropped").unwrap();
    let said = folder.join("pid");
    let script = format!("sleep 300 & echo $! > '{}'; wait", said.display());
    let processes = Processes::unrecorded();
    glave_ports::block_on!(async {
        let mut program = Program::new("sh");
        program.args(["-c", &script]);
        let waiting = processes.output(&mut program, AMPLE);
        // The caller waits only until the shell has said which sleep it started.
        let file = said.clone();
        let said_it = async move {
            while !std::fs::read_to_string(&file).is_ok_and(|pid| pid.ends_with('\n')) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::select! {
            ran = waiting => panic!("the shell ended first: {ran:?}"),
            () = said_it => {}
        }
    });
    let pid = std::fs::read_to_string(&said)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    gone(
        pid,
        "the sleep outlived the call that stopped waiting for it",
    );
}

#[test]
fn it_is_recorded_while_it_runs_and_forgotten_once_it_ends() {
    let run = glave_ports::testing::TempDir::new("waited-recorded").unwrap();
    let ledger = run.join("processes");
    let processes = Processes::recorded(&run).owned_by(Owner::Server);
    // The program says whether the ledger holds its entry, which is written once it started.
    let script = format!(
        "for _ in $(seq 1000); do ls '{}' 2>/dev/null | grep -q json && echo recorded && exit; \
         sleep 0.01; done",
        ledger.display()
    );
    let output =
        glave_ports::block_on!(processes.output(Program::new("sh").args(["-c", &script]), AMPLE))
            .unwrap();
    assert_eq!(output.stdout, b"recorded\n");
    assert_eq!(entries(&ledger), 0, "its entry goes once it ends");
}

#[test]
fn what_it_leaves_running_once_it_has_ended_is_left_be() {
    // As an opener leaves the browser it starts, its output elsewhere.
    let processes = Processes::unrecorded();
    let output = glave_ports::block_on!(processes.output(
        Program::new("sh").args(["-c", "sleep 300 >/dev/null 2>&1 & echo $!"]),
        AMPLE
    ))
    .unwrap();
    let pid: i32 = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let sleep = Pid::from_raw(pid).unwrap();
    let running = test_kill_process(sleep).is_ok();
    let _ = kill_process(sleep, Signal::KILL);
    assert!(running, "the sleep was stopped with the shell");
}
