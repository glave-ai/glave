//! `glob`: files whose name or path matches a pattern, newest first, leaving out what git
//! ignores.

use std::fmt::Write as _;

use glave_ports::computer::Glob;
use serde::Deserialize;

use super::ToolOutcome;
use super::workspace::Workspace;

/// Most files listed by one call.
const MAX_FILES: usize = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct GlobArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
}

pub(crate) async fn glob(workspace: &Workspace, args: GlobArgs) -> ToolOutcome {
    let glob = Glob {
        pattern: args.pattern.clone(),
        path: args.path.unwrap_or_else(|| ".".into()),
        limit: MAX_FILES,
    };
    let found = match workspace.glob(&glob).await {
        Ok(found) => found,
        Err(message) => return ToolOutcome::error(message),
    };
    if found.files.is_empty() {
        return ToolOutcome::ok(format!(
            "No files match {} in {}.",
            args.pattern, found.path
        ));
    }
    let mut out = found.files.join("\n");
    if found.total > u64::try_from(MAX_FILES).unwrap_or(u64::MAX) {
        let _ = write!(
            out,
            "\n[{MAX_FILES} of {} files shown, newest first. Narrow the pattern or path to see the rest.]",
            found.total
        );
    }
    ToolOutcome::ok(out)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use glave_ports::block_on;

    use super::*;

    fn run(dir: &Path, args: serde_json::Value) -> ToolOutcome {
        let args: GlobArgs = serde_json::from_value(args).unwrap();
        block_on!(async {
            let workspace = Workspace::open(dir, []).await.unwrap();
            glob(&workspace, args).await
        })
    }

    /// Writes `files` with modification times one minute apart, oldest first.
    fn project(name: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-glob-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let base = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        for (minute, path) in (0_u64..).zip(files) {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "x\n").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(base + Duration::from_mins(minute))
                .unwrap();
        }
        dir
    }

    #[test]
    fn matching_files_come_newest_first_without_ignored_ones() {
        let dir = project(
            "newest",
            &[
                ".gitignore",
                "src/old.rs",
                "notes.md",
                "src/deep/new.rs",
                "target/built.rs",
            ],
        );
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        let out = run(&dir, serde_json::json!({ "pattern": "*.rs" }));
        assert_eq!(out.text, "src/deep/new.rs\nsrc/old.rs");
        let none = run(
            &dir,
            serde_json::json!({ "pattern": "*.py", "path": "src" }),
        );
        assert_eq!(none.text, "No files match *.py in src.");
    }

    #[test]
    fn a_bad_pattern_is_an_error() {
        let dir = project("bad", &["a.rs"]);
        assert!(run(&dir, serde_json::json!({ "pattern": "[" })).is_error);
    }
}
