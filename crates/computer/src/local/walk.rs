//! Walking the project the way git sees it, for discovery: ignored files and the `.git` folder
//! are left out, and file patterns read the way models write them.

use std::path::{Path, PathBuf};

use glave_ports::EntryKind;
use globset::{GlobBuilder, GlobMatcher};
use ignore::{Walk, WalkBuilder};

fn walker(root: &Path, depth: Option<usize>) -> Walk {
    WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .follow_links(false)
        .max_depth(depth)
        .sort_by_file_path(Ord::cmp)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
}

/// Every file under `root` that git would not ignore, even outside a repository, in path
/// order. Hidden files such as `.github/` are included; the `.git` folder is not. A file
/// given as `root` is returned as is.
pub(crate) fn files(root: &Path) -> impl Iterator<Item = PathBuf> {
    walker(root, None)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(ignore::DirEntry::into_path)
}

/// The entries directly inside `dir` that git would not ignore, in path order, each with its
/// kind (links are not followed), and how many others were left out (not counting `.git`).
pub(crate) fn entries(dir: &Path) -> std::io::Result<(Vec<(PathBuf, EntryKind)>, usize)> {
    let all = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != ".git")
        .count();
    let names: Vec<(PathBuf, EntryKind)> = walker(dir, Some(1))
        .filter_map(Result::ok)
        .filter(|entry| entry.depth() == 1)
        .map(|entry| {
            let kind = match entry.file_type() {
                Some(kind) if kind.is_dir() => EntryKind::Folder,
                Some(kind) if kind.is_symlink() => EntryKind::Link,
                _ => EntryKind::File,
            };
            (entry.into_path(), kind)
        })
        .collect();
    let left_out = all.saturating_sub(names.len());
    Ok((names, left_out))
}

/// A file pattern as models write it. Without a slash (`*.rs`) it matches file names in any
/// folder; with one (`src/**/*.rs`) it matches paths from the folder searched.
pub(crate) struct Pattern {
    glob: GlobMatcher,
    by_name: bool,
}

impl Pattern {
    pub(crate) fn new(pattern: &str) -> Result<Self, String> {
        let pattern = pattern.trim().trim_start_matches("./");
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|e| format!("{pattern} is not a valid file pattern: {e}"))?
            .compile_matcher();
        Ok(Self {
            glob,
            by_name: !pattern.contains('/'),
        })
    }

    /// Whether the file at `relative` (to the folder searched) matches.
    pub(crate) fn matches(&self, relative: &Path) -> bool {
        if self.by_name {
            relative
                .file_name()
                .is_some_and(|name| self.glob.is_match(name))
        } else {
            self.glob.is_match(relative)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::paths::relative;

    pub(crate) fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-walk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, contents) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        dir
    }

    #[test]
    fn discovery_skips_what_git_ignores_even_without_a_repository() {
        let dir = project(
            "ignored",
            &[
                (".gitignore", "target/\n*.log\n"),
                ("src/main.rs", "fn main() {}\n"),
                ("target/out.rs", "built\n"),
                ("debug.log", "noise\n"),
                (".github/ci.yml", "on: push\n"),
                (".git/config", "[core]\n"),
            ],
        );
        let found: Vec<String> = files(&dir).map(|path| relative(&dir, &path)).collect();
        assert_eq!(found, [".github/ci.yml", ".gitignore", "src/main.rs"]);
        let (names, left_out) = entries(&dir).unwrap();
        let names: Vec<(String, EntryKind)> = names
            .iter()
            .map(|(path, kind)| (relative(&dir, path), *kind))
            .collect();
        assert_eq!(
            names,
            [
                (".github".to_owned(), EntryKind::Folder),
                (".gitignore".to_owned(), EntryKind::File),
                ("src".to_owned(), EntryKind::Folder)
            ]
        );
        assert_eq!(left_out, 2, "target/ and debug.log");
    }

    #[test]
    fn patterns_without_a_slash_match_names_in_any_folder() {
        let any_rust = Pattern::new("*.rs").unwrap();
        assert!(any_rust.matches(Path::new("src/deep/lib.rs")));
        assert!(!any_rust.matches(Path::new("src/notes.md")));
        let top_of_src = Pattern::new("./src/*.rs").unwrap();
        assert!(top_of_src.matches(Path::new("src/main.rs")));
        assert!(!top_of_src.matches(Path::new("src/deep/lib.rs")));
        let anywhere = Pattern::new("**/*.rs").unwrap();
        assert!(anywhere.matches(Path::new("main.rs")));
        assert!(Pattern::new("src/[").is_err());
    }
}
