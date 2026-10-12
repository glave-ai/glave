//! Locating the workspace root.

use std::path::{Path, PathBuf};

/// The workspace root: the nearest folder at or above the current one that holds the
/// workspace's `Cargo.toml` and the `xtask` crate.
///
/// Found at run time, not from where xtask was built: worktrees can share one build folder
/// (`CARGO_TARGET_DIR`), and a root fixed at build time would check whichever worktree built
/// xtask last.
pub fn root() -> Result<PathBuf, String> {
    let here =
        std::env::current_dir().map_err(|e| format!("could not read the current folder: {e}"))?;
    root_from(&here).ok_or_else(|| {
        format!(
            "no Glave workspace (a folder with Cargo.toml and xtask/) at or above {}",
            here.display()
        )
    })
}

/// The nearest folder at or above `start` holding `Cargo.toml` and `xtask/Cargo.toml`.
fn root_from(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file() && dir.join("xtask/Cargo.toml").is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::root_from;
    use std::path::Path;

    #[test]
    fn the_root_is_found_from_any_folder_inside_the_workspace() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace = here.parent().unwrap();
        assert_eq!(root_from(workspace).as_deref(), Some(workspace));
        assert_eq!(root_from(&here.join("src")).as_deref(), Some(workspace));
        assert_eq!(
            root_from(&workspace.join("crates/ports/src")).as_deref(),
            Some(workspace)
        );
    }

    #[test]
    fn no_root_is_found_outside_a_workspace() {
        assert_eq!(root_from(Path::new("/")), None);
    }
}
