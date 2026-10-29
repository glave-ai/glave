//! Process groups stopped whole: SIGTERM, a grace period, then SIGKILL to whatever is left,
//! returning once the group is empty.

use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};

/// How long a group gets between SIGTERM and SIGKILL.
pub(crate) const GRACE: Duration = Duration::from_secs(3);
/// How often a stopped group is checked for anything left in it.
const POLL: Duration = Duration::from_millis(10);

/// The group numbered `id`, if that is a process id.
pub(crate) fn of(id: i32) -> Option<Pid> {
    Pid::from_raw(id)
}

/// Whether anything is left in `group`, even a process waiting to be reaped.
pub(crate) fn exists(group: Pid) -> bool {
    test_kill_process_group(group).is_ok()
}

/// Sends `signal` to everything in `group`. A group already gone is not an error.
pub(crate) fn signal(group: Pid, signal: Signal) {
    let _ = kill_process_group(group, signal);
}

/// Resolves once nothing is left in `group`.
pub(crate) async fn emptied(group: Pid) {
    while exists(group) {
        tokio::time::sleep(POLL).await;
    }
}

/// Stops every group of `groups` side by side: SIGTERM to each, then, after [`GRACE`],
/// SIGKILL to those with anything left, and waits up to another grace for them to empty.
/// Returns the groups still not empty then: left behind, since a process stuck in the kernel
/// (a hung network mount) may never end, and glave moves on rather than wait for it.
pub(crate) async fn stop_all(groups: &[Pid]) -> Vec<Pid> {
    let left = |groups: &[Pid]| -> Vec<Pid> {
        groups
            .iter()
            .copied()
            .filter(|group| exists(*group))
            .collect()
    };
    let running = left(groups);
    for group in &running {
        signal(*group, Signal::TERM);
    }
    let all_emptied = async {
        for group in &running {
            emptied(*group).await;
        }
    };
    if tokio::time::timeout(GRACE, all_emptied).await.is_ok() {
        return Vec::new();
    }
    let stubborn = left(&running);
    for group in &stubborn {
        signal(*group, Signal::KILL);
    }
    let all_emptied = async {
        for group in &stubborn {
            emptied(*group).await;
        }
    };
    if tokio::time::timeout(GRACE, all_emptied).await.is_ok() {
        return Vec::new();
    }
    left(&stubborn)
}
