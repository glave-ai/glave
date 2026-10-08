//! `cargo xtask ci`: the full local gate, identical to what CI runs.

use crate::cmd;
use crate::lock::{self, Mode};

/// Runs every check in order and stops at the first failure.
pub fn run() -> Result<(), String> {
    let _guard = lock::acquire(Mode::Shared)?;
    step("format", || cmd::run("cargo", &["fmt", "--all", "--check"]))?;
    step("lint", || {
        cmd::run(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
        )
    })?;
    step("tests", || {
        cmd::run(
            "cargo",
            &[
                "nextest",
                "run",
                "--workspace",
                "--all-features",
                "--no-tests=pass",
            ],
        )
    })?;
    step("dependencies", || cmd::run("cargo", &["deny", "check"]))?;
    step("file sizes", crate::file_sizes::check)?;
    println!("\nxtask: all checks passed");
    Ok(())
}

fn step(name: &str, check: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    println!("\n== {name}");
    check().map_err(|err| format!("{name}: {err}"))
}
