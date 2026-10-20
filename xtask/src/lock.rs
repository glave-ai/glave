//! A machine-wide lock that keeps benchmarks from running alongside tests or each other.
//!
//! Test runs take the lock shared, so any number can run together. Benchmarks take it
//! exclusively, so they wait for every test run to finish and run alone, which keeps
//! wall-clock numbers free of interference.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;

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

/// Acquires the lock in `mode`, waiting (with a notice) if another run holds it.
pub(crate) fn acquire(mode: Mode) -> Result<Guard, String> {
    let path = path();
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|err| format!("could not open lock file {}: {err}", path.display()))?;
    let attempt = match mode {
        Mode::Shared => file.try_lock_shared(),
        Mode::Exclusive => file.try_lock(),
    };
    match attempt {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            eprintln!("xtask: waiting for another test or benchmark run to finish");
            let blocking = match mode {
                Mode::Shared => file.lock_shared(),
                Mode::Exclusive => file.lock(),
            };
            blocking.map_err(|err| format!("could not take the benchmark lock: {err}"))?;
        }
        Err(TryLockError::Error(err)) => {
            return Err(format!("could not take the benchmark lock: {err}"));
        }
    }
    Ok(Guard { _file: file })
}
