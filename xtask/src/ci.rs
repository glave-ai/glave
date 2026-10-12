//! `cargo xtask ci`: the full local gate, identical to what CI runs: the Rust checks, then the
//! web checks (`web.rs`), which need Node unless `GLAVE_SKIP_WEB=1` skips them.

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
    // Called directly: run as `cargo machete` under `cargo xtask`, it takes `machete` for a path.
    step("unused dependencies", || cmd::run("cargo-machete", &[]))?;
    step("file sizes", crate::file_sizes::check)?;
    step("api schema", || crate::schema::run(&["--check".to_owned()]))?;
    step("web", crate::web::check)?;
    println!("\nxtask: all checks passed");
    Ok(())
}

fn step(name: &str, check: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    println!("\n== {name}");
    check().map_err(|err| format!("{name}: {err}"))
}
