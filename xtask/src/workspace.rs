//! Locating the workspace root, and what Cargo says of the workspace.

use std::path::{Path, PathBuf};

/// The workspace root: the nearest folder at or above the current one whose `Cargo.toml`
/// declares a `[workspace]` and which holds the `xtask` crate.
///
/// Found at run time, not from where xtask was built: worktrees can share one build folder
/// (`CARGO_TARGET_DIR`), and a root fixed at build time would check whichever worktree built
/// xtask last.
pub fn root() -> Result<PathBuf, String> {
    let here =
        std::env::current_dir().map_err(|e| format!("could not read the current folder: {e}"))?;
    root_from(&here).ok_or_else(|| {
        format!(
            "no Glave workspace (a Cargo.toml with [workspace], and xtask/) at or above {}",
            here.display()
        )
    })
}

/// What `cargo metadata` says of the workspace's own packages.
pub fn metadata() -> Result<serde_json::Value, String> {
    let text = crate::cmd::output("cargo", &["metadata", "--format-version", "1", "--no-deps"])?;
    serde_json::from_str(&text)
        .map_err(|err| format!("`cargo metadata` wrote something not JSON: {err}"))
}

/// The nearest folder at or above `start` that is the workspace root.
fn root_from(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| is_root(dir))
        .map(Path::to_path_buf)
}

fn is_root(dir: &Path) -> bool {
    dir.join("xtask/Cargo.toml").is_file()
        && std::fs::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|manifest| manifest.lines().any(|line| line.trim() == "[workspace]"))
}

#[cfg(test)]
mod tests {
    use super::root_from;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// A fresh folder for one test, removed first if an earlier run left it.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-root-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn workspace_at(dir: &Path) {
        fs::create_dir_all(dir.join("xtask")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"xtask\"]\n",
        )
        .unwrap();
        fs::write(
            dir.join("xtask/Cargo.toml"),
            "[package]\nname = \"xtask\"\n",
        )
        .unwrap();
    }

    #[test]
    fn the_root_is_found_from_the_root_and_from_any_folder_inside() {
        let dir = scratch("inside");
        workspace_at(&dir);
        let deep = dir.join("crates/ports/src");
        fs::create_dir_all(&deep).unwrap();

        assert_eq!(root_from(&dir), Some(dir.clone()));
        assert_eq!(root_from(&dir.join("xtask")), Some(dir.clone()));
        assert_eq!(root_from(&deep), Some(dir.clone()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_crate_manifest_or_a_workspace_without_xtask_is_passed_over() {
        let dir = scratch("passed-over");
        workspace_at(&dir);
        // A member crate: a Cargo.toml and an xtask/ beside it, but no [workspace].
        let member = dir.join("crates/tool");
        fs::create_dir_all(member.join("xtask")).unwrap();
        fs::write(member.join("Cargo.toml"), "[package]\nname = \"tool\"\n").unwrap();
        fs::write(member.join("xtask/Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        // A workspace of its own with no xtask.
        let other = dir.join("evals/task");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("Cargo.toml"), "[workspace]\n").unwrap();

        assert_eq!(root_from(&member), Some(dir.clone()));
        assert_eq!(root_from(&other), Some(dir.clone()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_nearest_workspace_wins() {
        let outer = scratch("nearest");
        workspace_at(&outer);
        let inner = outer.join("worktrees/one");
        workspace_at(&inner);

        assert_eq!(root_from(&inner.join("xtask")), Some(inner.clone()));
        assert_eq!(root_from(&outer.join("worktrees")), Some(outer.clone()));
        fs::remove_dir_all(outer).unwrap();
    }

    #[test]
    fn no_root_is_found_outside_a_workspace() {
        let dir = scratch("outside");
        assert_eq!(root_from(&dir), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
