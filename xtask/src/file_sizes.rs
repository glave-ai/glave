//! `cargo xtask file-sizes`: enforce the source-file line limits from AGENTS.md.

use std::path::{Path, PathBuf};

/// Above this many lines a file gets a warning: time to consider splitting it.
const SOFT_LIMIT: usize = 400;
/// Above this many lines a file fails the check.
const HARD_LIMIT: usize = 800;
/// Directories scanned, relative to the workspace root.
const SCANNED_DIRS: &[&str] = &["crates", "xtask", "apps", "web"];
/// Extensions counted as source code.
const SOURCE_EXTENSIONS: &[&str] = &["rs", "ts"];
/// Directory names never scanned (build output, dependencies, generated code).
const SKIPPED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "generated",
    ".angular",
    "out-tsc",
    "test-results",
];

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

/// Scans the source tree, warns about long files, and fails on files over the hard limit.
pub(crate) fn check() -> Result<(), String> {
    let root = crate::workspace::root()?;
    let mut files = Vec::new();
    for dir in SCANNED_DIRS {
        collect(&root.join(dir), &mut files)?;
    }
    let mut failures = Vec::new();
    for path in &files {
        let lines = count_lines(path)?;
        let shown = path.strip_prefix(&root).unwrap_or(path).display();
        match classify(lines) {
            Verdict::Fine => {}
            Verdict::Warn => println!("warning: {shown}: {lines} lines (target ≤ {SOFT_LIMIT})"),
            Verdict::Fail => failures.push(format!("{shown}: {lines} lines")),
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

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries =
        std::fs::read_dir(dir).map_err(|err| format!("could not read {}: {err}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|err| format!("could not read {}: {err}", dir.display()))?
            .path();
        if path.is_dir() {
            if !has_name_in(&path, SKIPPED_DIRS) {
                collect(&path, out)?;
            }
        } else if has_extension_in(&path, SOURCE_EXTENSIONS) {
            out.push(path);
        }
    }
    Ok(())
}

fn has_name_in(path: &Path, names: &[&str]) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| names.contains(&name))
}

fn has_extension_in(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| extensions.contains(&ext))
}

fn count_lines(path: &Path) -> Result<usize, String> {
    std::fs::read_to_string(path)
        .map(|text| text.lines().count())
        .map_err(|err| format!("could not read {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{HARD_LIMIT, SOFT_LIMIT, Verdict, classify, has_extension_in, has_name_in};
    use std::path::Path;

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
    fn matches_source_extensions_only() {
        assert!(has_extension_in(Path::new("a/b.rs"), &["rs"]));
        assert!(!has_extension_in(Path::new("a/b.md"), &["rs"]));
        assert!(!has_extension_in(Path::new("a/rs"), &["rs"]));
    }

    #[test]
    fn skips_named_directories() {
        assert!(has_name_in(Path::new("crates/x/target"), &["target"]));
        assert!(!has_name_in(Path::new("crates/x/src"), &["target"]));
    }
}
