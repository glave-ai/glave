//! Whether a command spent its run held at the CPU limit, as Linux counts it for the cgroup
//! glave runs in (cgroup v2's `cpu.stat`, `throttled_usec`), read before and after it. A
//! cgroup counts only where something limits its CPU, a Pod's; where the file cannot be read
//! (macOS, cgroup v1, a sandboxed kernel that keeps none) nothing is counted. Memory is not
//! counted here: a container killed for memory goes whole, glave with it, which its server
//! sees instead (`specs/computers.md`, decision 8).

use std::path::{Path, PathBuf};

/// The shortest run whose time at the CPU limit counts: a command that runs for seconds is not
/// worth a bigger computer.
const CPU_RUN_MILLIS: u64 = 10_000;

/// The cgroup glave runs in, where its CPU time is counted.
pub(crate) fn cgroup() -> Option<PathBuf> {
    let own = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = own.lines().find_map(|line| line.strip_prefix("0::"))?;
    let folder = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
    folder.join("cpu.stat").is_file().then_some(folder)
}

/// The microseconds the cgroup at `folder` has spent held at its CPU limit, if it counts them.
pub(crate) async fn throttled(folder: Option<&Path>) -> Option<u64> {
    let stat = tokio::fs::read_to_string(folder?.join("cpu.stat"))
        .await
        .ok()?;
    field(&stat, "throttled_usec")
}

/// The number after `name` on its line of a cgroup file.
fn field(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == name).then(|| value.trim().parse().ok()).flatten()
    })
}

/// Whether a command whose run took `millis`, the cgroup held at its limit for `before` and
/// then `after` microseconds, spent at least half of a run of 10 seconds or more held there.
pub(crate) fn held(before: Option<u64>, after: Option<u64>, millis: u64) -> bool {
    let (Some(before), Some(after)) = (before, after) else {
        return false;
    };
    let held_millis = after.saturating_sub(before) / 1_000;
    millis >= CPU_RUN_MILLIS && held_millis.saturating_mul(2) >= millis
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cgroup_file_is_read_by_its_fields() {
        let cpu = "usage_usec 9\nnr_throttled 2\nthrottled_usec 41000\n";
        assert_eq!(field(cpu, "throttled_usec"), Some(41_000));
        assert_eq!(field(cpu, "nr_throttled"), Some(2));
        assert_eq!(field(cpu, "missing"), None);
    }

    #[test]
    fn a_run_held_at_the_cpu_limit_for_half_its_length_counts_once_it_is_long_enough() {
        assert!(held(Some(0), Some(6_000_000), 11_000));
        assert!(!held(Some(0), Some(5_000_000), 11_000), "less than half");
        assert!(
            !held(Some(0), Some(5_000_000), 9_000),
            "too short a run to grow for"
        );
        assert!(
            !held(None, Some(6_000_000), 11_000),
            "nothing counted before"
        );
    }
}
