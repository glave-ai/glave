//! The Rust sources the source checks read (`spawns`, `waits`): the tracked files under some
//! folders, tests aside (`tests/`, `benches/`, `tests.rs`, `*_tests.rs`, an inline `mod tests`
//! to the end of its file, and the test doubles in `testing.rs` and `testing/`), which may do
//! what they like.

use crate::{cmd, workspace};

/// Each non-test Rust source tracked under `folders`, by its path from the workspace root,
/// with its text.
pub(crate) fn read(folders: &[&str]) -> Result<Vec<(String, String)>, String> {
    let root = workspace::root()?;
    let listed = cmd::query("git", &[&["ls-files", "-z", "--"], folders].concat())?;
    listed
        .split('\0')
        .filter(|path| non_test(path))
        .map(|path| {
            std::fs::read_to_string(root.join(path))
                .map(|text| (path.to_owned(), text))
                .map_err(|err| format!("could not read {path}: {err}"))
        })
        .collect()
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

/// Each line of code in `text`, by number from 1, up to its inline tests: comment lines
/// aside, each trimmed at its start.
pub(crate) fn code(text: &str) -> Vec<(usize, &str)> {
    let mut found = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((n, line)) = lines.next() {
        let code = line.trim_start();
        let tests_follow = lines
            .peek()
            .is_some_and(|(_, next)| next.trim_start().starts_with("mod tests"));
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
        assert!(non_test("crates/cli/src/daemon.rs"));
        assert!(non_test("xtask/src/cmd.rs"));
        assert!(!non_test("crates/cli/tests/serve.rs"));
        assert!(!non_test("crates/computer/src/local/tests.rs"));
        assert!(!non_test("crates/computer/src/local/confinement_tests.rs"));
        assert!(!non_test("crates/cli/benches/budgets.rs"));
        assert!(!non_test("crates/ports/src/testing/http.rs"));
        assert!(!non_test("web/app/main.ts"));
    }
}
