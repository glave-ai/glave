//! `cargo xtask ci`: the full local gate, identical to what CI runs: the Rust checks, then the
//! web checks (`web.rs`), which need Node unless `GLAVE_SKIP_WEB=1` skips them.

use serde_json::Value;

use crate::cmd;
use crate::lock::{self, Mode};

/// The feature that builds the web app into glave. It needs the app built first, so the Rust
/// checks run with every feature but this one, and the web checks test it.
pub const BUILT_IN_APP: (&str, &str) = ("glave", "web");

/// Runs every check in order and stops at the first failure.
pub fn run() -> Result<(), String> {
    let _guard = lock::acquire(Mode::Shared)?;
    step("format", || cmd::run("cargo", &["fmt", "--all", "--check"]))?;
    let features = features_in(&crate::workspace::metadata()?)?;
    step("lint", || {
        cmd::run(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--features",
                &features,
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
                "--features",
                &features,
                "--no-tests=pass",
            ],
        )
    })?;
    step("dependencies", || cmd::run("cargo", &["deny", "check"]))?;
    // Called directly: run as `cargo machete` under `cargo xtask`, it takes `machete` for a path.
    step("unused dependencies", || cmd::run("cargo-machete", &[]))?;
    step("file sizes", crate::file_sizes::check)?;
    step("api schema", || crate::schema::run(&["--check".to_owned()]))?;
    step("release workflow", crate::release_workflow::check)?;
    step("web", crate::web::check)?;
    println!("\nxtask: all checks passed");
    Ok(())
}

fn step(name: &str, check: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    println!("\n== {name}");
    check().map_err(|err| format!("{name}: {err}"))
}

/// Every feature of every package in `cargo metadata`'s JSON as `--features` takes them
/// (`glave/otlp`), but the one that builds the web app in.
fn features_in(metadata: &Value) -> Result<String, String> {
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("`cargo metadata` named no packages")?;
    let mut features = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or("`cargo metadata` named a package without its name")?;
        let declared = package
            .get("features")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("`cargo metadata` gave {name} no features"))?;
        features.extend(
            declared
                .keys()
                .filter(|feature| (name, feature.as_str()) != BUILT_IN_APP)
                .map(|feature| format!("{name}/{feature}")),
        );
    }
    Ok(features.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_is_checked_but_the_built_in_app() {
        let metadata = serde_json::json!({"packages": [
            {"name": "glave", "features": {"otlp": [], "web": []}},
            {"name": "glave-keys", "features": {"keychain": [], "conformance": []}},
            {"name": "glave-event", "features": {}},
        ]});
        assert_eq!(
            features_in(&metadata),
            Ok("glave/otlp,glave-keys/conformance,glave-keys/keychain".to_owned())
        );
        assert!(features_in(&serde_json::json!({})).is_err());
    }
}
