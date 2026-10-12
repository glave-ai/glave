//! The web checks in `cargo xtask ci`: format, lint, type-check, licences and audit of what
//! ships, unit tests, the library and app builds, and the end-to-end tests of `web/`
//! (`npm run check`), served by the real `glave serve --http --web`, which this step builds.
//! They need Node 22.22.3 or later: without it the step fails, unless `GLAVE_SKIP_WEB=1` says
//! to skip it, as CI's Rust jobs do (its `web` job runs these).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// The oldest Node the web build supports (Angular's own minimum, `web/package.json`).
const MINIMUM: (u32, u32, u32) = (22, 22, 3);

/// Runs the web checks, or skips them when asked to.
pub fn check() -> Result<(), String> {
    if std::env::var_os("GLAVE_SKIP_WEB").is_some_and(|value| value == "1") {
        println!("web: skipped (GLAVE_SKIP_WEB=1)");
        return Ok(());
    }
    let version = node_version().ok_or(
        "the web checks need Node 22.22.3 or later, and none is installed \
         (set GLAVE_SKIP_WEB=1 to skip them)",
    )?;
    if version < MINIMUM {
        return Err(format!(
            "the web checks need Node {}.{}.{} or later, not {}.{}.{} \
             (set GLAVE_SKIP_WEB=1 to skip them)",
            MINIMUM.0, MINIMUM.1, MINIMUM.2, version.0, version.1, version.2
        ));
    }
    let root = crate::workspace::root()?;
    let web = root.join("web");
    if stale(&web) {
        npm(&web, &["ci"], None)?;
    }
    npm(&web, &["run", "e2e:install"], None)?;
    // The end-to-end tests run against the real server.
    crate::cmd::run("cargo", &["build", "-p", "glave"])?;
    let glave = target_directory()?.join("debug/glave");
    npm(&web, &["run", "check"], Some(&glave))
}

/// Where cargo builds this workspace: `CARGO_TARGET_DIR` or `build.target-dir` when set, else
/// the workspace's own `target/`, as `cargo metadata` reports it.
fn target_directory() -> Result<PathBuf, String> {
    let metadata =
        crate::cmd::output("cargo", &["metadata", "--format-version", "1", "--no-deps"])?;
    target_directory_in(&metadata)
}

/// The `target_directory` in `cargo metadata`'s JSON.
fn target_directory_in(metadata: &str) -> Result<PathBuf, String> {
    let value: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|err| format!("`cargo metadata` wrote something not JSON: {err}"))?;
    value
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| "`cargo metadata` named no target directory".to_owned())
}

/// Whether `web/node_modules` is missing or older than `package-lock.json`.
fn stale(web: &Path) -> bool {
    let modified = |path: &Path| -> Option<SystemTime> { path.metadata().ok()?.modified().ok() };
    match (
        modified(&web.join("package-lock.json")),
        modified(&web.join("node_modules/.package-lock.json")),
    ) {
        (Some(lock), Some(installed)) => lock > installed,
        _ => true,
    }
}

/// The installed Node's version, if there is one.
fn node_version() -> Option<(u32, u32, u32)> {
    let output = Command::new("node").arg("--version").output().ok()?;
    parse(String::from_utf8(output.stdout).ok()?.trim())
}

/// `v22.22.3` as `(22, 22, 3)`.
fn parse(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text
        .trim_start_matches('v')
        .split('.')
        .map(str::parse::<u32>);
    Some((
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    ))
}

fn npm(dir: &Path, args: &[&str], glave: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new("npm");
    command.args(args).current_dir(dir);
    if let Some(glave) = glave {
        command.env("GLAVE_BIN", glave);
    }
    let status = command
        .status()
        .map_err(|err| format!("could not start npm: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`npm {}` in web/ failed ({status})",
            args.join(" ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_versions_are_read_and_ordered() {
        assert_eq!(parse("v22.22.3"), Some((22, 22, 3)));
        assert_eq!(parse("24.15.0"), Some((24, 15, 0)));
        assert_eq!(parse("nope"), None);
        assert!(parse("v22.22.0") < Some(MINIMUM));
        assert!(parse("v24.1.0") > Some(MINIMUM));
    }

    #[test]
    fn the_target_directory_is_cargo_metadatas() {
        assert_eq!(
            target_directory_in(r#"{"target_directory":"/cache/glave-target","version":1}"#),
            Ok(PathBuf::from("/cache/glave-target"))
        );
        assert!(target_directory_in(r#"{"version":1}"#).is_err());
        assert!(target_directory_in("not json").is_err());
    }

    #[test]
    fn a_missing_install_is_stale() {
        let dir = std::env::temp_dir().join(format!("xtask-web-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        assert!(stale(&dir));
        std::fs::write(dir.join("node_modules/.package-lock.json"), "{}").unwrap();
        assert!(!stale(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
