//! `.env` files, read without a shell: which one is nearest, and what one sets.

use std::path::{Path, PathBuf};

/// The nearest `.env`: at or above `root`, else at the main checkout's root when `root` is a
/// git worktree.
pub(crate) fn nearest(root: &Path) -> Option<PathBuf> {
    nearest_from(root, || {
        let git = crate::cmd::query("git", &["rev-parse", "--git-common-dir"]).ok()?;
        Some(root.join(git.trim()))
    })
}

/// The nearest `.env` at or above `root`, else the one beside `main_git`: the main checkout's
/// `.git`, which its worktrees share.
fn nearest_from(root: &Path, main_git: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    root.ancestors()
        .map(|dir| dir.join(".env"))
        .find(|file| file.is_file())
        .or_else(|| {
            let file = main_git()?.parent()?.join(".env");
            file.is_file().then_some(file)
        })
}

/// What the `.env` file at `path` sets each of `names` to, for the names it sets.
pub(crate) fn read<'a>(path: &Path, names: &[&'a str]) -> Result<Vec<(&'a str, String)>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    Ok(values(&text, names))
}

/// The last value each of `names` has in `text`, when it is not empty.
fn values<'a>(text: &str, names: &[&'a str]) -> Vec<(&'a str, String)> {
    let pairs: Vec<(&str, &str)> = text.lines().filter_map(pair).collect();
    names
        .iter()
        .filter_map(|&name| {
            let (_, value) = pairs.iter().rev().find(|(key, _)| *key == name)?;
            (!value.is_empty()).then(|| (name, (*value).to_owned()))
        })
        .collect()
}

/// `KEY=value` on one line, `export ` allowed before it. None for a blank line, a comment or
/// a line with no `=`.
fn pair(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let line = line.strip_prefix("export ").unwrap_or(line);
    if line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), unquoted(value.trim())))
}

/// A value without the quotes around it, or without the ` #` comment after it when unquoted.
fn unquoted(value: &str) -> &str {
    ['"', '\'']
        .into_iter()
        .find_map(|quote| {
            let rest = value.strip_prefix(quote)?;
            rest.get(..rest.find(quote)?)
        })
        .unwrap_or_else(|| {
            value
                .split_once(" #")
                .map_or(value, |(before, _)| before.trim_end())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const NAMES: [&str; 2] = ["GLAVE_BASE_URL", "GLAVE_API_KEY"];

    fn set(text: &str) -> Vec<(&'static str, String)> {
        values(text, &NAMES)
    }

    #[test]
    fn a_line_is_a_key_and_its_value() {
        let text = "\
# the model endpoint
GLAVE_BASE_URL=http://localhost:4000/v1

export GLAVE_API_KEY=plain-words
OTHER=left alone
";
        assert_eq!(
            set(text),
            [
                ("GLAVE_BASE_URL", "http://localhost:4000/v1".to_owned()),
                ("GLAVE_API_KEY", "plain-words".to_owned()),
            ]
        );
    }

    #[test]
    fn quotes_and_comments_are_not_the_value() {
        for (line, value) in [
            ("GLAVE_API_KEY=\"double quoted\"", "double quoted"),
            ("GLAVE_API_KEY='single quoted'", "single quoted"),
            ("GLAVE_API_KEY=\"a # b\" # a note", "a # b"),
            ("GLAVE_API_KEY=unquoted # a note", "unquoted"),
            ("  GLAVE_API_KEY = spaced  ", "spaced"),
            ("GLAVE_API_KEY=\"never closed", "\"never closed"),
            ("GLAVE_API_KEY=a=b", "a=b"),
        ] {
            assert_eq!(set(line), [("GLAVE_API_KEY", value.to_owned())], "{line}");
        }
    }

    #[test]
    fn comments_blanks_empty_values_and_other_lines_set_nothing() {
        let text = "\
# GLAVE_BASE_URL=commented
#GLAVE_API_KEY=commented
GLAVE_BASE_URL=

not a pair
GLAVE_API_KEY_OLD=another name
";
        assert_eq!(set(text), []);
        assert_eq!(set(""), []);
    }

    #[test]
    fn the_last_value_wins() {
        let text = "GLAVE_API_KEY=first\r\nGLAVE_API_KEY=second\r\n";
        assert_eq!(set(text), [("GLAVE_API_KEY", "second".to_owned())]);
    }

    /// A fresh folder for one test, removed first if an earlier run left it.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-env-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_nearest_file_at_or_above_the_workspace_is_found() {
        let dir = scratch("nearest");
        let root = dir.join("checkout");
        fs::create_dir_all(&root).unwrap();
        fs::write(dir.join(".env"), "").unwrap();
        assert_eq!(nearest_from(&root, || None), Some(dir.join(".env")));
        fs::write(root.join(".env"), "").unwrap();
        assert_eq!(nearest_from(&root, || None), Some(root.join(".env")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_worktree_finds_the_main_checkouts_file() {
        let dir = scratch("worktree");
        let main = dir.join("main");
        let worktree = dir.join("worktree");
        fs::create_dir_all(main.join(".git")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        let main_git = || Some(main.join(".git"));
        assert_eq!(nearest_from(&worktree, main_git), None);
        fs::write(main.join(".env"), "").unwrap();
        assert_eq!(nearest_from(&worktree, main_git), Some(main.join(".env")));
        fs::write(worktree.join(".env"), "").unwrap();
        assert_eq!(
            nearest_from(&worktree, main_git),
            Some(worktree.join(".env")),
            "the worktree's own file comes first"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_is_read_and_a_missing_one_named() {
        let dir = scratch("read");
        let file = dir.join("endpoint.env");
        fs::write(&file, "GLAVE_BASE_URL=http://localhost:4000/v1\n").unwrap();
        assert_eq!(
            read(&file, &NAMES),
            Ok(vec![(
                "GLAVE_BASE_URL",
                "http://localhost:4000/v1".to_owned()
            )])
        );
        let missing = dir.join("missing.env");
        let err = read(&missing, &NAMES).unwrap_err();
        assert!(err.contains(&missing.display().to_string()), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }
}
