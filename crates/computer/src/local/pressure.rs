//! What a command ran short of, as Linux counts it for the cgroup glave runs in (cgroup v2): a
//! kill for running out of memory (`memory.events`, `oom_kill`) and time held at the CPU limit
//! (`cpu.stat`, `throttled_usec`), read before and after the command. Only a computer with
//! limits of its own, a Pod's, counts them; elsewhere, and where the files cannot be read (macOS,
//! cgroup v1, a sandboxed kernel that keeps none), nothing is counted and nothing is said.

use std::path::{Path, PathBuf};

use glave_ports::computer::{Ending, Pressure};

/// The shortest run whose time at the CPU limit counts: a command that runs for seconds is not
/// worth a bigger computer.
const CPU_RUN_MILLIS: u64 = 10_000;

/// What the cgroup has counted so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counted {
    oom_kills: u64,
    throttled_usec: u64,
}

/// The cgroup glave runs in, where it can be read.
pub(crate) fn cgroup() -> Option<PathBuf> {
    let own = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = own.lines().find_map(|line| line.strip_prefix("0::"))?;
    let folder = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
    folder.join("memory.events").is_file().then_some(folder)
}

/// What the cgroup at `folder` has counted, if it can be read.
pub(crate) async fn counted(folder: Option<&Path>) -> Option<Counted> {
    let folder = folder?;
    let memory = tokio::fs::read_to_string(folder.join("memory.events"))
        .await
        .ok()?;
    let cpu = tokio::fs::read_to_string(folder.join("cpu.stat"))
        .await
        .unwrap_or_default();
    Some(Counted {
        oom_kills: field(&memory, "oom_kill")?,
        throttled_usec: field(&cpu, "throttled_usec").unwrap_or(0),
    })
}

/// The number after `name` on its line of a cgroup file.
fn field(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == name).then(|| value.trim().parse().ok()).flatten()
    })
}

/// What a command that ended so after `millis` ran short of, between `before` and `after`: a
/// memory kill when the cgroup counted one and the command was killed by a signal, the CPU when
/// it spent at least half of a run of 10 seconds or more held at the limit.
pub(crate) fn pressure(
    before: Option<Counted>,
    after: Option<Counted>,
    ending: Ending,
    millis: u64,
) -> Pressure {
    let (Some(before), Some(after)) = (before, after) else {
        return Pressure::default();
    };
    let held_millis = after.throttled_usec.saturating_sub(before.throttled_usec) / 1_000;
    Pressure {
        memory: after.oom_kills > before.oom_kills && ending == Ending::Signalled,
        cpu: millis >= CPU_RUN_MILLIS && held_millis.saturating_mul(2) >= millis,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted(oom_kills: u64, throttled_usec: u64) -> Counted {
        Counted {
            oom_kills,
            throttled_usec,
        }
    }

    #[test]
    fn a_cgroup_file_is_read_by_its_fields() {
        let memory = "low 0\nhigh 0\nmax 12\noom 1\noom_kill 3\noom_group_kill 0\n";
        assert_eq!(field(memory, "oom_kill"), Some(3));
        assert_eq!(field(memory, "oom"), Some(1));
        let cpu = "usage_usec 9\nnr_throttled 2\nthrottled_usec 41000\n";
        assert_eq!(field(cpu, "throttled_usec"), Some(41_000));
        assert_eq!(field(cpu, "missing"), None);
    }

    #[test]
    fn a_kill_for_memory_and_a_run_held_at_the_cpu_limit_are_told_apart() {
        let killed = pressure(
            Some(counted(0, 0)),
            Some(counted(1, 0)),
            Ending::Signalled,
            500,
        );
        assert_eq!((killed.memory, killed.cpu), (true, false));
        let exited = pressure(
            Some(counted(0, 0)),
            Some(counted(1, 0)),
            Ending::Exited { code: 1 },
            500,
        );
        assert!(
            !exited.memory,
            "another process was killed; this one exited"
        );
        let held = pressure(
            Some(counted(0, 0)),
            Some(counted(0, 6_000_000)),
            Ending::Exited { code: 0 },
            11_000,
        );
        assert_eq!((held.memory, held.cpu), (false, true));
        let brief = pressure(
            Some(counted(0, 0)),
            Some(counted(0, 5_000_000)),
            Ending::Exited { code: 0 },
            9_000,
        );
        assert!(!brief.cpu, "too short a run to grow for");
        let unread = pressure(None, Some(counted(1, 0)), Ending::Signalled, 500);
        assert!(unread.is_none());
    }
}
