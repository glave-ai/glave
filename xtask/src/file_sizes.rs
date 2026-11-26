//! `cargo xtask file-sizes`: enforce the source-file line limits from AGENTS.md.

use crate::sources;

/// Above this many lines a file gets a warning: time to consider splitting it.
const SOFT_LIMIT: usize = 400;
/// Above this many lines a file fails the check.
const HARD_LIMIT: usize = 800;
/// Folders scanned, relative to the workspace root.
const SCANNED_DIRS: &[&str] = &["crates", "xtask", "web"];
/// Extensions counted as source code.
const SOURCE_EXTENSIONS: &[&str] = &["rs", "ts"];

/// How a file's length compares with the limits.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Fine,
    Warn,
    Fail,
}

fn classify(lines: usize) -> Verdict {
    if lines > HARD_LIMIT {
        Verdict::Fail
    } else if lines > SOFT_LIMIT {
        Verdict::Warn
    } else {
        Verdict::Fine
    }
}

/// Reads the tracked sources, warns about long files, and fails on files over the hard limit.
pub(crate) fn check() -> Result<(), String> {
    let files = sources::tracked(SCANNED_DIRS, counted)?;
    let mut failures = Vec::new();
    for (path, text) in &files {
        let lines = text.lines().count();
        match classify(lines) {
            Verdict::Fine => {}
            Verdict::Warn => println!("warning: {path}: {lines} lines (target ≤ {SOFT_LIMIT})"),
            Verdict::Fail => failures.push(format!("{path}: {lines} lines")),
        }
    }
    if failures.is_empty() {
        println!("{} source files within limits", files.len());
        Ok(())
    } else {
        Err(format!(
            "{} file(s) over {HARD_LIMIT} lines; split them:\n  {}",
            failures.len(),
            failures.join("\n  ")
        ))
    }
}

/// Whether `path` is counted: source code, generated code aside.
fn counted(path: &str) -> bool {
    !path.contains("/generated/")
        && std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&ext))
}

#[cfg(test)]
mod tests {
    use super::{HARD_LIMIT, SOFT_LIMIT, Verdict, classify, counted};

    #[test]
    fn limits_are_inclusive_at_the_boundary() {
        assert_eq!(classify(SOFT_LIMIT), Verdict::Fine);
        assert_eq!(classify(SOFT_LIMIT + 1), Verdict::Warn);
        assert_eq!(classify(HARD_LIMIT), Verdict::Warn);
        assert_eq!(classify(HARD_LIMIT + 1), Verdict::Fail);
    }

    #[test]
    fn empty_file_is_fine() {
        assert_eq!(classify(0), Verdict::Fine);
    }

    #[test]
    fn counts_sources_and_not_generated_code() {
        assert!(counted("crates/x/src/b.rs"));
        assert!(counted("web/app/src/main.ts"));
        assert!(!counted("crates/x/README.md"));
        assert!(!counted("crates/x/rs"));
        assert!(!counted("web/client/src/generated/api.ts"));
    }
}
