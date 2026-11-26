//! The tracked files the source checks read (`duplicates`, `file-sizes`, `secrets`, `spawns`,
//! `unused-pub`, `waits`), and which Rust sources are not tests' (`tests/`, `benches/`,
//! `tests.rs`, `*_tests.rs`, an inline `mod tests` to the end of its file, and the test doubles
//! in `testing.rs` and `testing/`), which may do what they like.

use std::collections::BTreeSet;
use std::io::ErrorKind;

use crate::{cmd, workspace};

/// Each file git tracks under `pathspecs` (every one when there are none) that `keep` takes, by
/// its path from the workspace root, with its text, in path order. A file that holds no text
/// (binary, a submodule's folder, or deleted but not yet staged) is left out.
pub(crate) fn tracked(
    pathspecs: &[&str],
    keep: impl Fn(&str) -> bool,
) -> Result<Vec<(String, String)>, String> {
    let root = workspace::root()?;
    let listed = cmd::query("git", &[&["ls-files", "-z", "--"], pathspecs].concat())?;
    // Once each: a file with a merge's conflict is listed once for each side.
    let paths: BTreeSet<&str> = listed
        .split('\0')
        .filter(|path| !path.is_empty() && keep(path))
        .collect();
    let mut read = Vec::with_capacity(paths.len());
    for path in paths {
        match std::fs::read_to_string(root.join(path)) {
            Ok(text) => read.push((path.to_owned(), text)),
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::InvalidData | ErrorKind::NotFound | ErrorKind::IsADirectory
                ) => {}
            Err(err) => return Err(format!("could not read {path}: {err}")),
        }
    }
    Ok(read)
}

/// Each non-test Rust source tracked under `folders`, by its path from the workspace root,
/// with its text.
pub(crate) fn read(folders: &[&str]) -> Result<Vec<(String, String)>, String> {
    tracked(folders, non_test)
}

/// Whether `path` is a Rust source that is not a test's.
pub(crate) fn non_test(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    std::path::Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "rs")
        && !path.contains("/testing/")
        && !path.contains("/tests/")
        && !path.contains("/benches/")
        && name != "testing.rs"
        && name != "tests.rs"
        && !name.ends_with("_tests.rs")
}

/// Each line of code in `text`, by number from 1, up to its inline tests (`#[cfg(test)]` then
/// `mod tests {`, to the end of the file; a `mod tests;` names another file and is read past):
/// comment lines aside, each trimmed at its start.
pub(crate) fn code(text: &str) -> Vec<(usize, &str)> {
    let mut found = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((n, line)) = lines.next() {
        let code = line.trim_start();
        let tests_follow = lines.peek().is_some_and(|(_, next)| {
            let next = next.trim_start();
            next.starts_with("mod tests") && next.trim_end().ends_with('{')
        });
        if code.starts_with("#[cfg(test)]") && tests_follow {
            break;
        }
        if !code.starts_with("//") {
            found.push((n + 1, code));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_and_comments_are_left_out() {
        let text = "fn go() {}\n// a comment\n    let x = 1;\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n";
        assert_eq!(code(text), [(1, "fn go() {}"), (3, "let x = 1;")]);
        let declared = "#[cfg(test)]\nmod tests;\n\nfn go() {}\n";
        assert_eq!(
            code(declared),
            [
                (1, "#[cfg(test)]"),
                (2, "mod tests;"),
                (3, ""),
                (4, "fn go() {}")
            ]
        );
        assert!(non_test("crates/cli/src/daemon.rs"));
        assert!(non_test("xtask/src/cmd.rs"));
        assert!(!non_test("crates/cli/tests/serve.rs"));
        assert!(!non_test("crates/computer/src/local/tests.rs"));
        assert!(!non_test("crates/computer/src/local/confinement_tests.rs"));
        assert!(!non_test("crates/session/benches/budgets.rs"));
        assert!(!non_test("crates/ports/src/testing/http.rs"));
        assert!(!non_test("web/app/main.ts"));
    }
}
