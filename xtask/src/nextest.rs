//! `cargo nextest run`, as `cargo xtask ci` runs the tests. On macOS every new program waits
//! before its first run while the system checks it, one at a time for the whole machine, so
//! beside other builds the wait for one program can pass a test's time limit (225 s, seen).
//! The check is per file, and cargo copies glave anew on every build there, so every run of
//! the tests pays it.
//!
//! So on macOS the tests are built first; the workspace's own programs they start (glave) are
//! run once, so the check happens here, where nothing times it; and the tests run from that
//! build, which nothing copies over again. Elsewhere the tests build and run in one go.
//!
//! The programs are left as the linker signed them. Signed ad hoc (`codesign --sign -`), the
//! check takes a shorter path, but signing writes the whole program anew, which on a disk
//! other builds share costs more than it saves: on an M3 building on a USB SSD beside other
//! agents' builds, a fresh debug glave (200 MB) first ran in 0.3 to 20 s as the linker signed
//! it and in 0.5 to 4.4 s signed, while signing it took 17 to 40 s, and 98 s while worktrees
//! were deleted (#1066). On an idle internal disk signing took a second and saved six (#529).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glave_process::Program;
use serde_json::Value;

use crate::cmd;

/// How long a program's first run may wait on macOS's check of it: seconds as a rule, minutes
/// beside other builds, as the check goes one program at a time for the whole machine.
const CHECKED_WITHIN: Duration = Duration::from_mins(5);

/// Runs the tests `build` selects (packages, features and targets, as cargo takes them), with
/// `options` (filters, `--no-tests`) and the variables `env`.
pub(crate) fn run(build: &[&str], options: &[&str], env: &[(&str, String)]) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return cmd::run_with(
            "cargo",
            &[&["nextest", "run"], build, options].concat(),
            env,
        );
    }
    let list = ["nextest", "list", "--list-type", "binaries-only"];
    let listed = cmd::output(
        "cargo",
        &[&list, build, &["--message-format", "json"]].concat(),
    )?;
    let binaries: Value = serde_json::from_str(&listed)
        .map_err(|err| format!("could not read the tests' programs nextest listed: {err}"))?;
    for program in programs(&binaries)? {
        first_run(&program)?;
    }
    // Named for this run: two runs of `cargo xtask ci` may share the target directory.
    let metadata = target(&binaries)?.join(format!("xtask-tests-{}.json", std::process::id()));
    std::fs::write(&metadata, listed)
        .map_err(|err| format!("could not write {}: {err}", metadata.display()))?;
    let reuse = [
        "nextest",
        "run",
        "--binaries-metadata",
        path_str(&metadata)?,
    ];
    let ran = cmd::run_with("cargo", &[&reuse, options].concat(), env);
    let _ = std::fs::remove_file(&metadata);
    ran
}

/// The target directory in nextest's list of the tests' programs.
fn target(binaries: &Value) -> Result<PathBuf, String> {
    binaries
        .pointer("/rust-build-meta/target-directory")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| "nextest's list of the tests' programs named no target directory".to_owned())
}

/// The workspace's own programs in nextest's list of the tests' programs, which tests start
/// (`glave`), where cargo put them.
fn programs(binaries: &Value) -> Result<Vec<PathBuf>, String> {
    let target = target(binaries)?;
    let programs = binaries
        .pointer("/rust-build-meta/non-test-binaries")
        .and_then(Value::as_object)
        .ok_or("nextest's list of the tests' programs named no other programs")?
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter(|binary| binary.get("kind").and_then(Value::as_str) == Some("bin-exe"))
        .filter_map(|binary| binary.get("path").and_then(Value::as_str))
        .map(|path| target.join(path))
        .collect();
    Ok(programs)
}

/// Runs `program` once, asking only for its help, and waits for it to end, so macOS checks it
/// now rather than while a test that starts it is timing it; says so when that took a while.
fn first_run(program: &Path) -> Result<(), String> {
    let started = Instant::now();
    Program::new(program)
        .arg("--help")
        .no_input()
        .output_blocking(CHECKED_WITHIN)
        .map_err(|err| format!("could not run {}: {err}", program.display()))?;
    let waited = started.elapsed();
    if waited > Duration::from_secs(1) {
        let name = program.file_name().unwrap_or(program.as_os_str());
        println!(
            "xtask: {}'s first run waited {} s on macOS's check of a new program",
            name.display(),
            waited.as_secs()
        );
    }
    Ok(())
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("{} is not UTF-8", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_programs_run_first_are_the_workspaces_own_where_cargo_put_them() {
        let binaries = serde_json::json!({
            "rust-build-meta": {
                "target-directory": "/cache/target",
                "non-test-binaries": {
                    "glave": [{"name": "glave", "kind": "bin-exe", "path": "debug/glave"}],
                    "crc-fast": [{"name": "crc_fast", "kind": "dylib", "path": "debug/deps/libcrc.dylib"}],
                },
            },
            "rust-binaries": {
                "glave::cli": {"kind": "test", "binary-path": "/cache/target/debug/deps/cli-1"},
            },
        });
        assert_eq!(
            programs(&binaries).unwrap(),
            [PathBuf::from("/cache/target/debug/glave")]
        );
        assert!(programs(&serde_json::json!({})).is_err());
    }
}
