//! When a process started, as the operating system says. A process id is reused once its
//! process is gone, so the id alone never says a process is the one glave started; the id
//! with its start time does.
//!
//! Linux: the start in clock ticks since boot (`/proc/<pid>/stat`), with the boot's id, so a
//! process after a restart never matches one before it. macOS: `ps`'s start time, to the
//! second, in UTC. Elsewhere none is known, so nothing is ever taken for a process glave
//! started.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A process's start, compared whole: equal starts with equal ids are the same process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Start(pub(crate) String);

/// The start of each process of `pids` that is running, or waiting to be reaped. Blocking.
pub(crate) fn of(pids: &[i32]) -> HashMap<i32, Start> {
    if pids.is_empty() {
        return HashMap::new();
    }
    platform(pids)
}

#[cfg(target_os = "linux")]
fn platform(pids: &[i32]) -> HashMap<i32, Start> {
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
    let boot = boot.trim();
    pids.iter()
        .filter_map(|&pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            Some((pid, Start(format!("{boot}/{}", ticks(&stat)?))))
        })
        .collect()
}

/// The start time (field 22) in a `/proc/<pid>/stat` line. The name before it is in
/// parentheses and may hold anything, so the fields are counted after its last `)`.
#[cfg(any(target_os = "linux", test))]
fn ticks(stat: &str) -> Option<&str> {
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(19)
}

/// How long `ps` has to list a few processes, which it does in milliseconds.
#[cfg(target_os = "macos")]
const PS_ANSWERS_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(target_os = "macos")]
fn platform(pids: &[i32]) -> HashMap<i32, Start> {
    let listed: Vec<String> = pids.iter().map(i32::to_string).collect();
    // Started directly rather than as a `Program`: reading a start is part of recording one.
    let mut ps = std::process::Command::new("/bin/ps");
    ps.args(["-o", "pid=,lstart=", "-p", &listed.join(",")])
        .env_clear()
        .env("LC_ALL", "C")
        .env("TZ", "UTC0")
        .stdin(std::process::Stdio::null());
    // `ps` exits 1 when some of them are gone, and still lists the rest. One that does not
    // answer in time lists none, so nothing is taken for a process glave started.
    match crate::within::output(&mut ps, PS_ANSWERS_WITHIN) {
        Ok(Some(output)) => listed_starts(&String::from_utf8_lossy(&output.stdout)),
        Ok(None) | Err(_) => HashMap::new(),
    }
}

/// Each line of `ps -o pid=,lstart=`: the id, then its start.
#[cfg(any(target_os = "macos", test))]
fn listed_starts(text: &str) -> HashMap<i32, Start> {
    text.lines()
        .filter_map(|line| {
            let (pid, start) = line.trim().split_once(char::is_whitespace)?;
            let start = start.trim();
            if start.is_empty() {
                return None;
            }
            Some((pid.parse().ok()?, Start(start.to_owned())))
        })
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform(_pids: &[i32]) -> HashMap<i32, Start> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_running_process_has_a_start_and_keeps_it() {
        let me = i32::try_from(std::process::id()).unwrap();
        let first = of(&[me]);
        assert!(first.contains_key(&me), "{first:?}");
        assert_eq!(of(&[me]), first);
        assert!(of(&[]).is_empty());
    }

    #[test]
    fn a_name_with_spaces_and_parentheses_does_not_move_the_start() {
        let stat = "42 (a) b (c) S 1 42 42 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 98765 1 2";
        assert_eq!(ticks(stat), Some("98765"));
        assert_eq!(ticks("42 (cut"), None);
    }

    #[test]
    fn ps_lines_give_each_id_its_start() {
        let listed = listed_starts("  7 Sun Oct 18 10:28:18 2026\n12 Mon Oct 19 01:02:03 2026\n\n");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[&7].0, "Sun Oct 18 10:28:18 2026");
        assert_eq!(listed[&12].0, "Mon Oct 19 01:02:03 2026");
    }
}
