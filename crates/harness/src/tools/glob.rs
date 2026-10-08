//! `glob`: files whose name or path matches a pattern, newest first, leaving out what git
//! ignores.

use std::fmt::Write as _;
use std::path::Path;
use std::time::SystemTime;

use serde::Deserialize;

use super::ToolOutcome;
use super::walk::{self, Pattern, shown};
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
    let start = match workspace.resolve(args.path.as_deref().unwrap_or(".")).await {
        Ok(path) => path,
        Err(message) => return ToolOutcome::error(message),
    };
    let pattern = match Pattern::new(&args.pattern) {
        Ok(pattern) => pattern,
        Err(message) => return ToolOutcome::error(message),
    };
    let root = workspace.root().to_owned();
    let place = shown(&root, &start);
    let listed = tokio::task::spawn_blocking(move || {
        let mut files: Vec<(SystemTime, String)> = walk::files(&start)
            .filter(|path| pattern.matches(path.strip_prefix(&start).unwrap_or(path)))
            .map(|path| (modified(&path), shown(&root, &path)))
            .collect();
        // Newest first: what changed recently is what a task is usually about.
        files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        files
    })
    .await;
    let Ok(files) = listed else {
        return ToolOutcome::error("The file search stopped unexpectedly.");
    };
    if files.is_empty() {
        return ToolOutcome::ok(format!("No files match {} in {place}.", args.pattern));
    }
    let total = files.len();
    let mut out = files
        .into_iter()
        .take(MAX_FILES)
        .map(|(_, path)| path)
        .collect::<Vec<_>>()
        .join("\n");
    if total > MAX_FILES {
        let _ = write!(
            out,
            "\n[{MAX_FILES} of {total} files shown, newest first. Narrow the pattern or path to see the rest.]"
        );
    }
    ToolOutcome::ok(out)
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn run(dir: &Path, args: serde_json::Value) -> ToolOutcome {
        let args: GlobArgs = serde_json::from_value(args).unwrap();
        block_on(async {
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
