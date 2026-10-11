//! The web checks in `cargo xtask ci`: format, lint, type-check, unit tests, the library and app
//! builds and the end-to-end smoke tests of `web/` (`npm run check`). They need Node; without
//! it, or with a Node older than Angular needs, the step is skipped with a message, so the Rust
//! gate stays usable on any machine. `GLAVE_SKIP_WEB=1` skips it too, as CI's Rust job does:
//! its own `web` job runs these.

use std::process::Command;

/// The oldest Node the web build supports (Angular's own minimum, `web/package.json`).
const MINIMUM: (u32, u32, u32) = (22, 22, 3);

/// Runs the web checks, or says why they are skipped.
pub fn check() -> Result<(), String> {
    if std::env::var_os("GLAVE_SKIP_WEB").is_some() {
        println!("web: skipped (GLAVE_SKIP_WEB is set)");
        return Ok(());
    }
    let Some(version) = node_version() else {
        println!("web: skipped: Node is not installed (the web checks need Node 22.22.3 or later)");
        return Ok(());
    };
    if version < MINIMUM {
        println!(
            "web: skipped: Node {}.{}.{} is older than {}.{}.{}, which Angular needs",
            version.0, version.1, version.2, MINIMUM.0, MINIMUM.1, MINIMUM.2
        );
        return Ok(());
    }
    let web = crate::workspace::root()?.join("web");
    if !web.join("node_modules").is_dir() {
        npm(&web, &["ci"])?;
    }
    npm(&web, &["run", "e2e:install"])?;
    npm(&web, &["run", "check"])
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

fn npm(dir: &std::path::Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("npm")
        .args(args)
        .current_dir(dir)
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
}
