//! A machine-wide lock that keeps benchmarks from running alongside tests or each other.
//!
//! Test runs take the lock shared, so any number can run together. Benchmarks take it
//! exclusively, so they wait for every test run to finish and run alone, which keeps
//! wall-clock numbers free of interference.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a run waits for others to finish before it gives up: well past the longest of
/// them (the full gate, the benchmarks, an eval), so one that still holds it then is stuck.
const WAITS_WITHIN: Duration = Duration::from_hours(2);

/// How often a waiting run tries the lock again.
const AGAIN: Duration = Duration::from_secs(1);

/// How a command shares the machine with other test and benchmark runs.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mode {
    /// Test runs: many may hold the lock at once.
    Shared,
    /// Benchmark runs: one holder, no test runs alongside it.
    Exclusive,
}

/// A held lock, released when dropped.
#[derive(Debug)]
pub(crate) struct Guard {
    _file: File,
}

fn path() -> PathBuf {
    std::env::temp_dir().join("glave-bench.lock")
}

/// Acquires the lock in `mode`, waiting (with a notice) if another run holds it, for up to
/// [`WAITS_WITHIN`].
pub(crate) fn acquire(mode: Mode) -> Result<Guard, String> {
    take(&path(), mode, WAITS_WITHIN)
}

/// Takes the lock at `path` in `mode`, waiting up to `limit` for whoever holds it.
fn take(path: &Path, mode: Mode, limit: Duration) -> Result<Guard, String> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|err| format!("could not open lock file {}: {err}", path.display()))?;
    let started = Instant::now();
    let mut said = false;
    loop {
        let attempt = match mode {
            Mode::Shared => file.try_lock_shared(),
            Mode::Exclusive => file.try_lock(),
        };
        match attempt {
            Ok(()) => return Ok(Guard { _file: file }),
            Err(TryLockError::WouldBlock) if started.elapsed() < limit => {
                if !said {
                    eprintln!("xtask: waiting for another test or benchmark run to finish");
                    said = true;
                }
                std::thread::sleep(AGAIN.min(limit));
            }
            Err(TryLockError::WouldBlock) => {
                return Err(format!(
                    "another test or benchmark run held the lock for {}; stop it, or try \
                     again once it ends",
                    glave_ports::spoken(limit)
                ));
            }
            Err(TryLockError::Error(err)) => {
                return Err(format!("could not take the benchmark lock: {err}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_held_past_the_limit_is_given_up_saying_for_how_long() {
        let dir = glave_ports::testing::TempDir::new("xtask-lock").unwrap();
        let path = dir.join("bench.lock");
        let held = take(&path, Mode::Exclusive, WAITS_WITHIN).unwrap();
        let waited = take(&path, Mode::Shared, Duration::from_millis(20)).unwrap_err();
        assert_eq!(
            waited,
            "another test or benchmark run held the lock for 20 ms; stop it, or try again once \
             it ends"
        );
        drop(held);
        assert!(take(&path, Mode::Shared, Duration::from_millis(20)).is_ok());
    }
}
