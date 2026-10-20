//! `cargo nextest run`, as `cargo xtask ci` runs the tests. On macOS every new program waits
//! on the system's check before its first run, and one that carries only the signature the
//! linker gives it counts as unsigned, which takes the check's slow path: a debug glave's
//! first run took 7 s on an idle M3, against 1.2 s signed ad hoc (`codesign --sign -`).
//! Cargo copies glave anew on every build there, so every run of the tests pays it. So on
//! macOS the tests are built first, the workspace's own programs they start (glave) are signed
//! ad hoc, and the tests run from that build, which nothing copies over again. The test
//! binaries are left as the linker signed them: they pay once per link, and signing them all
//! cost about what it saved. Elsewhere the tests build and run in one go.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::cmd;

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
        sign(&program)?;
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

/// Signs `program` ad hoc if it carries only the linker's signature.
fn sign(program: &Path) -> Result<(), String> {
    let shown = codesign(&["--display", "--verbose=1"], program)?;
    if !shown.status.success() || !linker_signed(&String::from_utf8_lossy(&shown.stderr)) {
        return Ok(());
    }
    let signed = codesign(&["--sign", "-", "--force"], program)?;
    if signed.status.success() {
        Ok(())
    } else {
        Err(format!(
            "could not sign {} ad hoc: {}",
            program.display(),
            String::from_utf8_lossy(&signed.stderr).trim()
        ))
    }
}

/// `codesign` with `args` on `program`, its output kept.
fn codesign(args: &[&str], program: &Path) -> Result<std::process::Output, String> {
    Command::new("codesign")
        .args(args)
        .arg(program)
        .output()
        .map_err(|err| format!("could not start `codesign`: {err}"))
}

/// Whether `codesign --display`'s description of a signature says only the linker signed it:
/// `flags=0x20002(adhoc,linker-signed)`.
fn linker_signed(description: &str) -> bool {
    description
        .lines()
        .filter(|line| line.starts_with("CodeDirectory "))
        .any(|line| line.contains("linker-signed"))
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("{} is not UTF-8", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_programs_signed_are_the_workspaces_own_where_cargo_put_them() {
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

    #[test]
    fn only_the_linkers_own_signature_is_replaced() {
        let linker = "Format=Mach-O thin (arm64)\n\
                      CodeDirectory v=20400 size=1437295 flags=0x20002(adhoc,linker-signed) \
                      hashes=44912+0 location=embedded\nSignature=adhoc\n";
        assert!(linker_signed(linker));
        let ad_hoc = "CodeDirectory v=20400 size=342186 flags=0x2(adhoc) hashes=10687+2 \
                      location=embedded\n";
        assert!(!linker_signed(ad_hoc));
        assert!(!linker_signed("Executable=/cache/linker-signed/glave\n"));
    }
}
