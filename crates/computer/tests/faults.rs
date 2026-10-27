//! The link to a remote computer, faulted on purpose: dropped, stalled, cut off, spliced,
//! duplicated, reordered and corrupted messages, and a computer killed mid-command. Whatever
//! happens, every call ends within its deadline (no hang), nothing the client asked for is
//! done twice, and an answer only ever reaches the call whose id it carries; garbage is a
//! protocol error that closes the connection. Time is paused: a stall costs nothing.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_computer::{CancelSwitch, RemoteComputers};
use glave_ports::BoxFuture;
use glave_ports::computer::{Command, ComputerError, Computers, Ending, FileWrite, Output};
use proptest::prelude::*;
use tokio::time::Instant;

use crate::faulty::{Fault, Planned, Way, faulty};
use crate::ledger::{Ledger, computer, workspace};

/// A runtime on paused time: it jumps ahead whenever every task waits on the clock.
fn paused<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(future)
}

/// One thing a test asks of the computer; `n` makes what it writes or runs unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Write(usize),
    Run(usize),
    Slow(usize),
    Read(usize),
    Status,
}

impl Op {
    /// The longest it may take: the client's deadline for it.
    fn deadline(self) -> Duration {
        match self {
            Self::Run(_) => Duration::from_secs(30 + 60),
            Self::Slow(_) => Duration::from_mins(3),
            Self::Write(_) | Self::Read(_) | Self::Status => Duration::from_secs(300),
        }
    }

    /// What the ledger counts when it is done.
    fn done_as(self) -> Option<String> {
        match self {
            Self::Write(n) => Some(format!("w{n}.txt=written {n}")),
            Self::Run(n) => Some(format!("run {n}")),
            Self::Slow(n) => Some(format!("slow {n}")),
            Self::Read(_) | Self::Status => None,
        }
    }
}

#[derive(Default)]
struct Text(Mutex<String>);

impl Output for Text {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push_str(text);
        Box::pin(std::future::ready(()))
    }
}

/// Does `op`, checking what came back is its own answer. `Err` only for a failure the
/// link caused.
async fn perform(remote: &RemoteComputers, op: Op) -> Result<(), ComputerError> {
    match op {
        Op::Write(n) => {
            let file = FileWrite::new(
                format!("w{n}.txt"),
                glave_ports::computer::Bytes::new(format!("written {n}")),
            );
            remote.write(workspace(), &[file]).await
        }
        Op::Run(n) | Op::Slow(n) => {
            let (script, timeout) = match op {
                Op::Slow(_) => (format!("slow {n}"), 120),
                _ => (format!("run {n}"), 30),
            };
            let mut command = Command::new(script.clone());
            command.timeout_secs = Some(timeout);
            let text = Text::default();
            let finished = remote
                .run(workspace(), &command, &text, &CancelSwitch::default())
                .await?;
            assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{op:?}");
            let printed = text.0.lock().unwrap().clone();
            for line in printed.lines() {
                assert_eq!(line, format!("ran {script}"), "another command's output");
            }
            Ok(())
        }
        Op::Read(n) => {
            let read = remote.read(workspace(), &format!("r{n}.txt")).await?;
            assert_eq!(read.as_slice(), format!("seeded {n}").as_bytes(), "{op:?}");
            Ok(())
        }
        Op::Status => {
            let info = remote.status(computer()).await?;
            assert_eq!(info.id, computer());
            Ok(())
        }
    }
}

/// What became of one op.
struct Outcome {
    op: Op,
    took: Duration,
    result: Result<(), ComputerError>,
}

/// Does `ops` all at once across a link with `faults`; every op ends, so this does.
async fn scenario(ops: &[Op], faults: &[Planned]) -> (Arc<Ledger>, Vec<Outcome>) {
    let ledger = Arc::new(Ledger::default());
    for n in 0..ops.len() {
        ledger.seed(&format!("r{n}.txt"), &format!("seeded {n}"));
    }
    let (remote, link) = faulty(Arc::clone(&ledger), faults);
    let remote = Arc::new(remote);
    let mut running = tokio::task::JoinSet::new();
    for &op in ops {
        let remote = Arc::clone(&remote);
        running.spawn(async move {
            let started = Instant::now();
            let result = perform(&remote, op).await;
            Outcome {
                op,
                took: started.elapsed(),
                result,
            }
        });
    }
    let mut outcomes = Vec::new();
    while let Some(outcome) = running.join_next().await {
        outcomes.push(outcome.unwrap());
    }
    drop(link);
    (ledger, outcomes)
}

/// The invariants every schedule must keep.
fn holds(ledger: &Ledger, outcomes: &[Outcome]) {
    for Outcome { op, took, result } in outcomes {
        assert!(
            *took <= op.deadline() + Duration::from_secs(2),
            "{op:?} took {took:?}, past its deadline"
        );
        if let Some(done) = op.done_as() {
            let times = ledger.times(&done);
            assert!(times <= 1, "{op:?} was done {times} times");
            if result.is_ok() {
                assert_eq!(times, 1, "{op:?} succeeded without being done");
            }
        }
        if let Err(error) = result {
            let ComputerError::Unreachable(message) = error else {
                panic!("{op:?} failed with something other than the link: {error:?}");
            };
            assert!(
                message.contains("does not send it again") || message.contains("was not sent"),
                "{op:?}: the error must say whether it may have run: {message}"
            );
        }
    }
}

fn run(ops: &[Op], faults: &[Planned]) -> (Arc<Ledger>, Vec<Outcome>) {
    paused(async {
        let ended = tokio::time::timeout(Duration::from_secs(3_600), scenario(ops, faults));
        let (ledger, outcomes) = ended.await.expect("every call ends");
        holds(&ledger, &outcomes);
        (ledger, outcomes)
    })
}

fn at(way: Way, at: usize, fault: Fault) -> Planned {
    Planned { way, at, fault }
}

fn message(outcome: &Outcome) -> String {
    match &outcome.result {
        Err(ComputerError::Unreachable(message)) => message.clone(),
        other => panic!("expected the link to fail {:?}: {other:?}", outcome.op),
    }
}

#[test]
fn a_request_delivered_twice_is_done_once_and_answered_once() {
    let (ledger, outcomes) = run(&[Op::Write(0)], &[at(Way::Out, 0, Fault::Duplicate)]);
    assert!(outcomes[0].result.is_ok());
    assert_eq!(ledger.times("w0.txt=written 0"), 1);
    let (ledger, outcomes) = run(&[Op::Run(0)], &[at(Way::Out, 0, Fault::Duplicate)]);
    assert!(outcomes[0].result.is_ok());
    assert_eq!(ledger.times("run 0"), 1);
}

#[test]
fn an_answer_delivered_twice_reaches_its_call_once() {
    let (_, outcomes) = run(
        &[Op::Read(0), Op::Read(1)],
        &[at(Way::Back, 0, Fault::Duplicate)],
    );
    assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
}

#[test]
fn answers_out_of_order_still_reach_their_own_calls() {
    let (_, outcomes) = run(
        &[Op::Read(0), Op::Read(1), Op::Status],
        &[at(Way::Back, 0, Fault::Reorder)],
    );
    assert!(outcomes.iter().all(|outcome| outcome.result.is_ok()));
}

#[test]
fn a_stalled_answer_fails_its_call_at_its_deadline_and_the_connection_lives_on() {
    paused(async {
        let ledger = Arc::new(Ledger::default());
        ledger.seed("r0.txt", "seeded 0");
        ledger.seed("r1.txt", "seeded 1");
        let stall = at(Way::Back, 0, Fault::Stall(Duration::from_mins(7)));
        let (remote, _link) = faulty(Arc::clone(&ledger), &[stall]);
        let started = Instant::now();
        let late = remote.read(workspace(), "r0.txt").await;
        assert_eq!(started.elapsed(), Duration::from_secs(300));
        assert!(
            matches!(&late, Err(ComputerError::Unreachable(m)) if m.contains("did not answer files/read within 5 minutes")),
            "{late:?}"
        );
        // The stalled answer arrives later and is dropped; the next call gets its own.
        let next = remote.read(workspace(), "r1.txt").await.unwrap();
        assert_eq!(next.as_slice(), b"seeded 1");
        assert!(remote.closed().is_none());
    });
}

#[test]
fn a_computer_that_does_not_say_hello_fails_in_seconds_not_minutes() {
    paused(async {
        let stall = at(Way::Back, 0, Fault::Stall(Duration::from_mins(7)));
        let (remote, _link) = faulty(Arc::new(Ledger::default()), &[stall]);
        let started = Instant::now();
        let late = remote.check_protocol().await;
        assert_eq!(started.elapsed(), Duration::from_secs(30));
        assert!(
            matches!(&late, Err(ComputerError::Unreachable(m)) if m.contains("did not answer server/discover within 30 seconds")),
            "{late:?}"
        );
    });
}

#[test]
fn a_dropped_connection_fails_the_call_saying_it_may_have_run_and_sends_nothing_more() {
    let (ledger, outcomes) = run(&[Op::Write(0)], &[at(Way::Back, 0, Fault::Drop)]);
    assert!(message(&outcomes[0]).contains("may or may not have done it"));
    assert_eq!(ledger.times("w0.txt=written 0"), 1, "it was done, once");
    let (ledger, outcomes) = run(&[Op::Write(0)], &[at(Way::Out, 0, Fault::Drop)]);
    assert!(message(&outcomes[0]).contains("may or may not have done it"));
    assert_eq!(ledger.times("w0.txt=written 0"), 0, "it never arrived");
    paused(async {
        let ledger = Arc::new(Ledger::default());
        let (remote, link) = faulty(ledger, &[at(Way::Out, 0, Fault::Drop)]);
        assert!(remote.status(computer()).await.is_err());
        drop(link);
        let after = remote.status(computer()).await;
        assert!(
            matches!(&after, Err(ComputerError::Unreachable(m)) if m.contains("was not sent")),
            "{after:?}"
        );
    });
}

#[test]
fn a_command_running_when_the_link_drops_fails_saying_it_may_have_run() {
    // The command's first output is held 5 seconds, and the link drops a second in: the
    // computer stops the command, and its answer never crosses the link that is gone.
    let (ledger, outcomes) = run(
        &[Op::Slow(0), Op::Write(1), Op::Write(2)],
        &[
            at(Way::Back, 0, Fault::Stall(Duration::from_secs(5))),
            at(Way::Out, 1, Fault::Stall(Duration::from_secs(1))),
            at(Way::Out, 2, Fault::Drop),
        ],
    );
    let slow = outcomes.iter().find(|outcome| outcome.op == Op::Slow(0));
    let said = slow.map(message).unwrap_or_default();
    assert!(said.contains("may or may not have done it"), "{said}");
    assert_eq!(ledger.times("slow 0"), 1);
}

#[test]
fn a_message_cut_off_or_run_into_the_next_is_a_protocol_error() {
    let (_, outcomes) = run(&[Op::Read(0)], &[at(Way::Back, 0, Fault::Truncate)]);
    assert!(
        message(&outcomes[0]).contains("cut off"),
        "{}",
        message(&outcomes[0])
    );
    let (_, outcomes) = run(
        &[Op::Read(0), Op::Read(1)],
        &[at(Way::Back, 0, Fault::Splice)],
    );
    for outcome in &outcomes {
        assert!(
            message(outcome).contains("not JSON"),
            "{}",
            message(outcome)
        );
    }
    // A request run into the next: the computer cannot read it, says so without an id,
    // and the client closes the connection rather than guess which call it was.
    let (ledger, outcomes) = run(
        &[Op::Write(0), Op::Write(1)],
        &[at(Way::Out, 0, Fault::Splice)],
    );
    assert_eq!(ledger.times("w0.txt=written 0"), 0);
    assert_eq!(ledger.times("w1.txt=written 1"), 0);
    for outcome in &outcomes {
        assert!(
            message(outcome).contains("could not read a request"),
            "{}",
            message(outcome)
        );
    }
}

#[test]
fn a_corrupted_byte_is_rejected_and_never_misread() {
    for which in [0, 7, 30, 1_000] {
        let (_, outcomes) = run(&[Op::Read(0)], &[at(Way::Back, 0, Fault::Corrupt(which))]);
        assert!(
            message(&outcomes[0]).contains("not JSON"),
            "{}",
            message(&outcomes[0])
        );
        let (ledger, outcomes) = run(&[Op::Write(0)], &[at(Way::Out, 0, Fault::Corrupt(which))]);
        assert!(outcomes[0].result.is_err());
        assert_eq!(
            ledger.times("w0.txt=written 0"),
            0,
            "a damaged request never runs"
        );
    }
}

#[test]
fn a_request_damaged_while_a_command_runs_fails_the_command_as_the_link_not_as_cancelled() {
    // The stall lets the command start before the damaged request comes. The computer cannot
    // read that request as text, so it stops reading and stops the command; the client hears
    // why first and closes the connection, never taking the stopped command for one it cancelled.
    let (ledger, outcomes) = run(
        &[Op::Slow(0), Op::Write(1), Op::Write(2)],
        &[
            at(Way::Out, 1, Fault::Stall(Duration::from_secs(1))),
            at(Way::Out, 2, Fault::Corrupt(0)),
        ],
    );
    let slow = outcomes.iter().find(|outcome| outcome.op == Op::Slow(0));
    let said = slow.map(message).unwrap_or_default();
    assert!(said.contains("could not read a request"), "{said}");
    assert_eq!(ledger.times("slow 0"), 1);
    assert_eq!(
        ledger.times("w2.txt=written 2"),
        0,
        "a damaged request never runs"
    );
}

#[test]
fn a_computer_killed_mid_command_fails_the_command_without_a_hang() {
    // The command's first output goes back; the computer dies before passing it on.
    let (ledger, outcomes) = run(&[Op::Slow(0)], &[at(Way::Back, 0, Fault::Kill)]);
    assert!(message(&outcomes[0]).contains("may or may not have done it"));
    assert!(outcomes[0].took < Op::Slow(0).deadline());
    assert_eq!(ledger.times("slow 0"), 1);
}

fn fault() -> impl Strategy<Value = Fault> {
    prop_oneof![
        Just(Fault::Drop),
        (1_u64..900).prop_map(|secs| Fault::Stall(Duration::from_secs(secs))),
        Just(Fault::Truncate),
        Just(Fault::Splice),
        Just(Fault::Duplicate),
        Just(Fault::Reorder),
        (0_usize..400).prop_map(Fault::Corrupt),
        Just(Fault::Kill),
    ]
}

fn planned() -> impl Strategy<Value = Planned> {
    (any::<bool>(), 0_usize..10, fault()).prop_map(|(out, at, fault)| Planned {
        way: if out { Way::Out } else { Way::Back },
        at,
        fault,
    })
}

fn op() -> impl Strategy<Value = u8> {
    0_u8..5
}

fn ops(kinds: &[u8]) -> Vec<Op> {
    kinds
        .iter()
        .enumerate()
        .map(|(n, kind)| match kind {
            0 => Op::Write(n),
            1 => Op::Run(n),
            2 => Op::Slow(n),
            3 => Op::Read(n),
            _ => Op::Status,
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any few faults, anywhere, under a few calls at once: no call hangs, nothing is
    /// done twice, and no call gets another's answer.
    #[test]
    fn no_schedule_of_faults_hangs_repeats_or_misattributes(
        kinds in prop::collection::vec(op(), 1..6),
        faults in prop::collection::vec(planned(), 0..4),
    ) {
        run(&ops(&kinds), &faults);
    }
}
