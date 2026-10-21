use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::Instant;

use glave_ports::testing::{HUNG, TempDir};
use rustix::process::{Pid, Signal, kill_process};

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

/// The helper running now.
fn helper_pid(lifeline: &Lifeline) -> u32 {
    let mut state = lifeline.lock();
    state.helper.as_mut().unwrap().process.id().unwrap()
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
        let (one, two) = (lifeline.tie(101).unwrap(), lifeline.tie(102).unwrap());
        drop(one);
        let said = "+101\n+102\n-101\n";
        assert_eq!(told(&folder, first, said).await, said);

        // The helper goes; the next program glave starts finds it gone, starts another and
        // tells it everything glave still holds.
        let pid = Pid::from_raw(i32::try_from(first).unwrap()).unwrap();
        kill_process(pid, Signal::KILL).unwrap();
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
        let three = lifeline.tie(103).unwrap();
        let second = helper_pid(&lifeline);
        assert_ne!(second, first);
        drop(two);
        let said = "+102\n+103\n-102\n";
        assert_eq!(told(&folder, second, said).await, said);
        drop(three);
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
