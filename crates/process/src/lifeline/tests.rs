use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::Instant;

use glave_ports::testing::{HUNG, TempDir};
use rustix::process::{Pid, Signal, kill_process, test_kill_process};

use super::*;
use crate::testing::Orphan;
use crate::{Processes, Program};

/// A stand-in for glave as the helper: a script that says it is ready, then keeps what it
/// reads in `told-<its pid>` beside itself.
fn fake_helper(folder: &Path) -> PathBuf {
    let script = folder.join("helper");
    let ready = String::from_utf8(READY.to_vec()).unwrap();
    let text = format!("#!/bin/sh\nprintf '{ready}'\nexec cat > \"${{0%/*}}/told-$$\"\n");
    std::fs::write(&script, text).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// A program at glave's path that is no helper (another version): it says something else, then
/// reads what it is given, keeping its number in `imposter` beside itself.
fn imposter(folder: &Path) {
    let text = "#!/bin/sh\necho $$ > \"${0%/*}/imposter\"\nprintf 'glave-lifeline 2\\n'\n\
                exec cat > /dev/null\n";
    std::fs::write(folder.join("helper"), text).unwrap();
}

/// A stand-in helper that says it is ready only once a file `go` is beside it.
fn gated_helper(folder: &Path) {
    let ready = String::from_utf8(READY.to_vec()).unwrap();
    let text = format!(
        "#!/bin/sh\nwhile [ ! -e \"${{0%/*}}/go\" ]; do sleep 0.01; done\nprintf '{ready}'\n\
         exec cat > \"${{0%/*}}/told-$$\"\n"
    );
    std::fs::write(folder.join("helper"), text).unwrap();
}

/// A stand-in helper whose first start stops itself once it says it is ready, before it reads
/// anything, so its pipe fills whatever the scheduler does; every later start reads, as
/// [`fake_helper`]'s does. A stop sent from outside could come too late: on macOS one sent as
/// the helper starts sometimes leaves it reading (#1044).
fn stopping_helper(folder: &Path) -> PathBuf {
    let script = folder.join("helper");
    let ready = String::from_utf8(READY.to_vec()).unwrap();
    let text = format!(
        "#!/bin/sh\nprintf '{ready}'\nmkdir \"${{0%/*}}/stopped\" 2>/dev/null && kill -STOP $$\n\
         exec cat > \"${{0%/*}}/told-$$\"\n"
    );
    std::fs::write(&script, text).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// The helper running now.
fn helper_pid(lifeline: &Lifeline) -> u32 {
    let mut state = lifeline.lock();
    state.helper.as_mut().unwrap().process.id().unwrap()
}

/// Kills the helper `pid`, and returns once glave would find it gone.
async fn killed(lifeline: &Lifeline, pid: u32) {
    let leader = Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
    kill_process(leader, Signal::KILL).unwrap();
    let deadline = Instant::now() + HUNG;
    while lifeline
        .lock()
        .helper
        .as_mut()
        .unwrap()
        .process
        .try_wait()
        .unwrap()
        .is_none()
    {
        assert!(Instant::now() < deadline, "the helper did not end");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// What the helper `pid` read, once it has read `wanted` (or the guard against a hang ran out).
async fn told(folder: &Path, pid: u32, wanted: &str) -> String {
    let path = folder.join(format!("told-{pid}"));
    let deadline = Instant::now() + HUNG;
    loop {
        let read = std::fs::read_to_string(&path).unwrap_or_default();
        if read == wanted || Instant::now() > deadline {
            return read;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn the_helper_stops_what_glave_still_held_once_its_end_closes() {
    let (held, let_go) = (Orphan::start(), Orphan::start());
    let lines = format!("+{}\n+{}\n-{}\n", held.pid, let_go.pid, let_go.pid);
    // A slice reads to its end as the pipe does once glave ends, however it ends.
    let still = helper::follow(lines.as_bytes());
    helper::stop(&still);
    assert!(
        held.ended(),
        "what glave still held when it ended is stopped"
    );
    assert!(let_go.alive(), "what glave let go of is not the helper's");
}

#[test]
fn glave_tells_its_helper_what_it_holds_and_a_new_helper_all_it_still_holds() {
    let folder = TempDir::new("lifeline").unwrap();
    glave_ports::block_on!(async {
        let lifeline = Arc::new(Lifeline::start(fake_helper(&folder)).await.unwrap());
        let first = helper_pid(&lifeline);
        let (one, two) = (
            lifeline.tie(101).await.unwrap(),
            lifeline.tie(102).await.unwrap(),
        );
        drop(one);
        let said = "+101\n+102\n-101\n";
        assert_eq!(told(&folder, first, said).await, said);

        // The helper goes; the next program glave starts finds it gone, starts another and
        // tells it everything glave still holds.
        killed(&lifeline, first).await;
        let three = lifeline.tie(103).await.unwrap();
        let second = helper_pid(&lifeline);
        assert_ne!(second, first);
        drop(two);
        let said = "+102\n+103\n-102\n";
        assert_eq!(told(&folder, second, said).await, said);
        drop(three);
    });
}

#[test]
fn a_helper_started_again_that_does_not_say_it_is_ready_is_refused_until_one_does() {
    let folder = TempDir::new("lifeline-again").unwrap();
    glave_ports::block_on!(async {
        let lifeline = Arc::new(Lifeline::start(fake_helper(&folder)).await.unwrap());
        let first = helper_pid(&lifeline);
        let held = lifeline.tie(101).await.unwrap();

        // What is at glave's path is no helper now (a rebuild or an upgrade put another version
        // there), and the helper goes. Two programs start together: the first starts what is
        // there, the second tells it and waits for it too. Both are refused, not left untied.
        imposter(&folder);
        killed(&lifeline, first).await;
        let other = tokio::spawn({
            let lifeline = Arc::clone(&lifeline);
            async move { lifeline.tie(103).await.err() }
        });
        let refused = [lifeline.tie(102).await.err(), other.await.unwrap()];
        for refused in refused {
            assert!(
                matches!(refused, Some(ProcessError::Lifeline { .. })),
                "{refused:?}"
            );
        }
        assert!(
            lifeline.lock().helper.is_none(),
            "it is not kept as the helper"
        );
        assert_eq!(
            lifeline.lock().live,
            BTreeSet::from([101]),
            "102 and 103 are let go"
        );
        let pid = std::fs::read_to_string(folder.join("imposter")).unwrap();
        let pid = Pid::from_raw(pid.trim().parse().unwrap()).unwrap();
        let deadline = Instant::now() + HUNG;
        while test_kill_process(pid).is_ok() {
            assert!(Instant::now() < deadline, "what is not a helper is stopped");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // A helper at glave's path again: the next program starts it, telling it all glave holds.
        fake_helper(&folder);
        let again = lifeline.tie(104).await.unwrap();
        let second = helper_pid(&lifeline);
        let said = "+101\n+104\n";
        assert_eq!(told(&folder, second, said).await, said);
        drop((held, again));
    });
}

#[test]
fn a_program_left_waiting_by_a_start_given_up_starts_the_helper_itself() {
    let folder = TempDir::new("lifeline-given-up").unwrap();
    glave_ports::block_on!(async {
        let lifeline = Arc::new(Lifeline::start(fake_helper(&folder)).await.unwrap());
        let first = helper_pid(&lifeline);
        let held = lifeline.tie(101).await.unwrap();

        // The helper goes; the one at glave's path now answers only once let. One program
        // starts it and waits; another tells it and waits too.
        gated_helper(&folder);
        killed(&lifeline, first).await;
        let starting = tokio::spawn({
            let lifeline = Arc::clone(&lifeline);
            async move { lifeline.tie(102).await.map(drop) }
        });
        tokio::task::yield_now().await;
        let waiting = tokio::spawn({
            let lifeline = Arc::clone(&lifeline);
            async move { lifeline.tie(103).await }
        });
        tokio::task::yield_now().await;

        // The first program's start is given up before its helper answers: the helper is
        // stopped, and the program left waiting starts another rather than be refused.
        starting.abort();
        std::fs::write(folder.join("go"), "").unwrap();
        let again = waiting.await.unwrap().unwrap();
        assert!(starting.await.unwrap_err().is_cancelled());
        let second = helper_pid(&lifeline);
        let said = "+101\n+103\n";
        assert_eq!(told(&folder, second, said).await, said);
        drop((held, again));
    });
}

#[test]
fn once_glave_reaps_a_leader_its_group_alone_is_held_until_let_go() {
    let folder = TempDir::new("lifeline-reaped").unwrap();
    glave_ports::block_on!(async {
        let processes = Processes::unrecorded()
            .with_lifeline(fake_helper(&folder))
            .await
            .unwrap();
        let helper = helper_pid(processes.lifeline().unwrap());
        let mut child = processes.spawn(&mut Program::new("true")).await.unwrap();
        let leader = child.id().unwrap();
        child.wait().await.unwrap();
        let said = format!("+{leader}\n~{leader}\n");
        assert_eq!(told(&folder, helper, &said).await, said);
        drop(child);
        let said = format!("{said}-{leader}\n");
        assert_eq!(told(&folder, helper, &said).await, said);
    });
}

#[test]
fn a_program_glave_waits_for_is_held_while_it_runs_and_let_go_once_it_ends() {
    let folder = TempDir::new("lifeline-waited").unwrap();
    glave_ports::block_on!(async {
        let processes = Processes::unrecorded()
            .with_lifeline(fake_helper(&folder))
            .await
            .unwrap();
        let helper = helper_pid(processes.lifeline().unwrap());
        let output = processes
            .output(Program::new("sh").args(["-c", "echo $$"]), HUNG)
            .await
            .unwrap();
        let leader = String::from_utf8(output.stdout).unwrap();
        let leader = leader.trim();
        let said = format!("+{leader}\n~{leader}\n-{leader}\n");
        assert_eq!(told(&folder, helper, &said).await, said);
    });
}

#[test]
fn the_helper_stops_the_group_of_a_leader_glave_reaped() {
    // What a reaped leader left runs on in its group: here, a process that leads a group, which
    // the helper reaches through the group alone.
    let left = Orphan::start();
    let held = helper::Held::from([(left.pid, true)]);
    helper::stop(&held);
    assert!(left.ended(), "what the leader left in its group is stopped");
}

#[test]
fn a_program_that_cannot_be_the_helper_is_refused_before_anything_starts() {
    let folder = TempDir::new("no-lifeline").unwrap();
    for helper in [folder.join("missing"), PathBuf::from("/usr/bin/true")] {
        let refused =
            glave_ports::block_on!(Processes::unrecorded().with_lifeline(&helper)).unwrap_err();
        assert!(
            matches!(refused, ProcessError::Lifeline { .. }),
            "{}: {refused}",
            helper.display()
        );
    }
}

#[test]
fn a_helper_that_reads_nothing_never_stalls_glave_and_is_replaced() {
    let folder = TempDir::new("lifeline-stopped").unwrap();
    glave_ports::block_on!(async {
        let lifeline = Arc::new(Lifeline::start(stopping_helper(&folder)).await.unwrap());
        let first = helper_pid(&lifeline);
        let held = lifeline.tie(101).await.unwrap();

        // The helper stops itself, not gone: it never reads, and glave's lines fill its pipe.
        // Programs keep starting and ending without waiting for it, until one finds it full.
        let stopped = Pid::from_raw(i32::try_from(first).unwrap()).unwrap();
        let deadline = Instant::now() + HUNG;
        let mut leader = 1000;
        while lifeline
            .lock()
            .helper
            .as_ref()
            .is_some_and(|helper| helper.launch == 1)
        {
            assert!(Instant::now() < deadline, "glave never found the pipe full");
            drop(lifeline.tie(leader).await.unwrap());
            leader += 1;
        }

        // The stopped helper is killed, so it can never wake to stop what glave holds; the next
        // program has another, told what glave holds.
        while test_kill_process(stopped).is_ok() {
            assert!(Instant::now() < deadline, "the stopped helper is killed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let again = lifeline.tie(1).await.unwrap();
        let second = helper_pid(&lifeline);
        let path = folder.join(format!("told-{second}"));
        let mut read = String::new();
        let holds = |read: &str| read.contains("+1\n") && read.contains("+101\n");
        while !holds(&read) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
            read = std::fs::read_to_string(&path).unwrap_or_default();
        }
        assert!(holds(&read), "{read}");
        drop((held, again));
    });
}
