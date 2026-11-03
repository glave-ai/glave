//! Whether a command spent its run held at the CPU limit: whether it used about all the CPU that
//! limit allows, as Linux counts it for the cgroup glave runs in (cgroup v2's `usage_usec` in
//! `cpu.stat`, and `cpu.max`), read before and after it. Time spent held (`throttled_usec`) is
//! not read: a Pod's cgroup has the same quota as its container's and takes a share of that time
//! that varies from run to run and can't be seen from inside (#1170). Where the files cannot be
//! read (macOS, cgroup v1, a sandboxed kernel that keeps none) or nothing limits the cgroup
//! (`max`), nothing is counted. Memory is not counted here: a container killed for memory goes
//! whole, glave with it, which its server sees instead (`specs/computers.md`, decision 8).

use std::num::{NonZeroU64, NonZeroU128};
use std::path::{Path, PathBuf};

/// The shortest run whose time at the CPU limit counts: a command that runs for seconds is not
/// worth a bigger computer.
const CPU_RUN_MILLIS: u64 = 10_000;

/// The percentage of what the limit allows that a run must use to count as held there: short of
/// all of it, since a run's clock also covers its waits before its work starts and after it ends.
const HELD_PERCENT: u128 = 90;

/// The cgroup glave runs in, where its CPU time is counted.
pub(crate) fn cgroup() -> Option<PathBuf> {
    let own = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = own.lines().find_map(|line| line.strip_prefix("0::"))?;
    let folder = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
    folder.join("cpu.stat").is_file().then_some(folder)
}

/// The CPU a cgroup has used so far, and its limit at that moment, in microseconds.
pub(crate) struct Counted {
    used: u64,
    quota: NonZeroU64,
    period: NonZeroU64,
}

impl Counted {
    /// What `cpu.stat` and `cpu.max` say, where they count CPU and set a limit on it.
    fn read(stat: &str, max: &str) -> Option<Self> {
        let mut limit = max.split_whitespace();
        Some(Self {
            used: field(stat, "usage_usec")?,
            // `max` is no quota: it does not parse, so nothing is counted.
            quota: limit.next()?.parse().ok()?,
            period: limit.next()?.parse().ok()?,
        })
    }

    /// The microseconds of CPU this limit allows over `millis` of a run.
    fn allowed_usec(&self, millis: u64) -> u128 {
        u128::from(self.quota.get()) * u128::from(millis) * 1_000 / NonZeroU128::from(self.period)
    }
}

/// The cgroup at `folder`'s count, where it keeps one and has a limit.
pub(crate) async fn counted(folder: Option<&Path>) -> Option<Counted> {
    let folder = folder?;
    let stat = tokio::fs::read_to_string(folder.join("cpu.stat"))
        .await
        .ok()?;
    let max = tokio::fs::read_to_string(folder.join("cpu.max"))
        .await
        .ok()?;
    Counted::read(&stat, &max)
}

/// The number after `name` on its line of a cgroup file.
fn field(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == name).then(|| value.trim().parse().ok()).flatten()
    })
}

/// Whether a command whose run took `millis`, the cgroup counted `before` and `after` it, used
/// about all its limit allows over a run of 10 seconds or more. A limit that changed during the
/// run is judged at the run's end, the larger, since a computer only grows while it is awake: a
/// run that used only what the old limit allowed has no need of another growth.
pub(crate) fn held(before: Option<Counted>, after: Option<Counted>, millis: u64) -> bool {
    let (Some(before), Some(after)) = (before, after) else {
        return false;
    };
    let used = after.used.saturating_sub(before.used);
    millis >= CPU_RUN_MILLIS && u128::from(used) * 100 >= after.allowed_usec(millis) * HELD_PERCENT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter of a CPU, as a person's computer wakes.
    const QUARTER: &str = "25000 100000\n";
    /// Half a CPU, as it first grows.
    const HALF: &str = "50000 100000\n";

    /// A count of `used_usec` under the limit `max`.
    fn count(used_usec: u64, max: &str) -> Option<Counted> {
        Counted::read(&format!("usage_usec {used_usec}\n"), max)
    }

    #[test]
    fn a_cgroup_file_is_read_by_its_fields() {
        let cpu = "usage_usec 9\nnr_throttled 2\nthrottled_usec 41000\n";
        assert_eq!(field(cpu, "throttled_usec"), Some(41_000));
        assert_eq!(field(cpu, "nr_throttled"), Some(2));
        assert_eq!(field(cpu, "missing"), None);
    }

    #[test]
    fn a_run_using_nine_tenths_of_its_limit_counts_once_it_is_long_enough() {
        // A quarter of a CPU allows 3 seconds of it in 12, of which 90% is 2.7.
        assert!(held(count(0, QUARTER), count(2_700_000, QUARTER), 12_000));
        assert!(
            !held(count(0, QUARTER), count(2_699_999, QUARTER), 12_000),
            "less than 90%"
        );
        assert!(
            !held(count(0, QUARTER), count(2_700_000, QUARTER), 9_999),
            "too short a run to grow for"
        );
        assert!(
            held(count(1_000_000, QUARTER), count(3_700_000, QUARTER), 12_000),
            "only what this run used counts"
        );
    }

    /// Numbers of the sizes #1170 saw at a quarter of a CPU over 12 seconds, whichever cgroup
    /// took the time spent held: time held says nothing, and CPU used says it every time.
    #[test]
    fn time_spent_held_neither_makes_nor_breaks_a_run() {
        let at = |usage: u64, throttled: u64| {
            let stat = format!("usage_usec {usage}\nthrottled_usec {throttled}\n");
            Counted::read(&stat, QUARTER)
        };
        assert!(
            held(at(0, 0), at(3_030_000, 3_521_786), 12_000),
            "the time held counted on the run that failed on CI, too little for the old rule"
        );
        assert!(held(at(0, 0), at(3_054_000, 33_272_000), 12_000));
        assert!(
            !held(at(0, 0), at(1_000_000, 33_272_000), 12_000),
            "much time held, little CPU used"
        );
    }

    #[test]
    fn a_limit_that_grows_during_a_run_is_judged_at_its_end() {
        // Half a CPU allows 6 seconds in 12, of which 90% is 5.4.
        let grew = |used| held(count(0, QUARTER), count(used, HALF), 12_000);
        assert!(!grew(3_000_000), "all the old limit allows, and no more");
        assert!(grew(5_400_000), "held at the new limit too");
    }

    #[test]
    fn no_limit_or_nothing_counted_is_never_held() {
        let free = "max 100000\n";
        assert!(count(5, free).is_none(), "no quota");
        assert!(!held(count(0, free), count(9_000_000, free), 12_000));
        assert!(
            !held(count(0, free), count(9_000_000, QUARTER), 12_000),
            "a limit set during the run"
        );
        assert!(
            !held(count(0, QUARTER), count(9_000_000, free), 12_000),
            "a limit lifted during the run"
        );
        assert!(
            !held(None, count(9_000_000, QUARTER), 12_000),
            "nothing counted before"
        );
        assert!(count(5, "").is_none(), "no limit file's contents");
        assert!(
            Counted::read("throttled_usec 5\n", QUARTER).is_none(),
            "no CPU counted"
        );
    }

    #[test]
    fn a_quota_or_period_of_nothing_counts_nothing() {
        assert!(count(0, "25000 0\n").is_none());
        assert!(count(0, "0 100000\n").is_none());
    }
}
