//! Paths as a command names them, resolved without touching the disk: `~` and `$HOME`, `.` and
//! `..`, relative to the folder the command runs in. Symbolic links are not followed, so a path
//! is judged by what it says, never by what the disk happens to hold.

use std::path::{Component, Path, PathBuf};

/// A program's name without its folder: `/bin/rm` and `\rm` are `rm`.
pub(crate) fn basename(name: &str) -> &str {
    let name = name.strip_prefix('\\').unwrap_or(name);
    name.rsplit('/').next().unwrap_or(name)
}

/// `text` as an absolute path from `cwd`, with `~` and `$HOME` taken as `home`.
pub(crate) fn resolve(cwd: &Path, text: &str, home: Option<&Path>) -> PathBuf {
    let homed = ["~", "$HOME", "${HOME}"].iter().find_map(|prefix| {
        let rest = text.strip_prefix(prefix)?;
        (rest.is_empty() || rest.starts_with('/')).then_some(rest)
    });
    let joined = match (homed, home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
        _ => cwd.join(text),
    };
    normalize(&joined)
}

/// `.` and `..` folded away, as far as the path itself says.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Whether removing `path` would remove something no project command should: the root, a
/// top-level folder, the home folder, or the project itself or any folder holding it.
pub(crate) fn critical(path: &Path, project: &Path, home: Option<&Path>) -> bool {
    path.components().count() <= 2
        || home.is_some_and(|home| path == home)
        || project.starts_with(path)
}

/// Whether `path` is inside `folder`, `folder` itself included.
pub(crate) fn inside(path: &Path, folder: &Path) -> bool {
    path.starts_with(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_from_the_folder_and_home() {
        let cwd = Path::new("/work/app");
        let home = Some(Path::new("/home/ana"));
        assert_eq!(resolve(cwd, "src/../lib", home), Path::new("/work/app/lib"));
        assert_eq!(resolve(cwd, "~/.ssh", home), Path::new("/home/ana/.ssh"));
        assert_eq!(resolve(cwd, "$HOME", home), Path::new("/home/ana"));
        assert_eq!(resolve(cwd, "/tmp/x", home), Path::new("/tmp/x"));
        assert_eq!(resolve(cwd, "~other", home), Path::new("/work/app/~other"));
        assert_eq!(basename("/bin/rm"), "rm");
        assert_eq!(basename("\\rm"), "rm");
    }

    #[test]
    fn critical_paths_are_the_root_top_folders_home_and_what_holds_the_project() {
        let project = Path::new("/work/app");
        let home = Some(Path::new("/home/ana"));
        for path in ["/", "/usr", "/home/ana", "/work", "/work/app"] {
            assert!(critical(Path::new(path), project, home), "{path}");
        }
        for path in ["/work/app/target", "/home/ana/notes", "/tmp/build"] {
            assert!(!critical(Path::new(path), project, home), "{path}");
        }
    }
}
