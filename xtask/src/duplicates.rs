//! `cargo xtask duplicates`: no new copy-paste (AGENTS.md, "Search before writing": where a
//! second copy would go, share the first instead). The tracked Rust under `crates/` and
//! `xtask/` and the TypeScript under `web/` (generated code aside) are read as lines of code:
//! each trimmed, with blank lines, lines of brackets alone, comments, and imports and module
//! declarations left out, since those are meant to look alike. Any [`WINDOW`] such lines in a
//! row found in two places start a copy, which runs on while both places go on alike; a copy
//! of fewer than [`LEAST_TOKENS`] tokens is too small to share. Each copy is named by a
//! fingerprint of its text, and the places it is in.
//!
//! The copies there were when the check began are in `duplicates/baseline.txt`. A copy not
//! there fails, and so does an entry no longer found, so the list only shrinks: a change that
//! shares one takes its line out, and one that edits or moves a listed copy writes the line
//! the check prints in its place.

mod clones;

use std::collections::BTreeSet;

use crate::{cmd, workspace};
use clones::Copied;

/// Lines of code in a row that, found twice, start a copy.
const WINDOW: usize = 6;

/// The fewest tokens (words and marks) a copy has to be counted.
const LEAST_TOKENS: usize = 70;

/// The copies the check lets stand, from the workspace root.
const BASELINE: &str = "xtask/src/duplicates/baseline.txt";

/// Fails on a copy of code [`BASELINE`] does not list, and on an entry of it no longer found.
pub(crate) fn check() -> Result<(), String> {
    let root = workspace::root()?;
    let listed = cmd::query("git", &["ls-files", "-z", "--", "crates", "xtask", "web"])?;
    // Once each: a file with a merge's conflict is listed once for each side.
    let paths: BTreeSet<&str> = listed.split('\0').filter(|path| scanned(path)).collect();
    let mut sources = Vec::new();
    for path in paths {
        match std::fs::read_to_string(root.join(path)) {
            Ok(text) => sources.push((path.to_owned(), text)),
            // A file deleted but not yet staged holds no code.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(format!("could not read {path}: {err}")),
        }
    }
    let baseline = std::fs::read_to_string(root.join(BASELINE))
        .map_err(|err| format!("could not read {BASELINE}: {err}"))?;
    let found = clones::find(&sources, WINDOW, LEAST_TOKENS);
    let problems = against(&found, &baseline);
    if problems.is_empty() {
        println!(
            "xtask: no new copy-paste in {} sources (the {} older copies are each listed in \
             {BASELINE})",
            sources.len(),
            found.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} problem(s) with copied code:\n  {}",
            problems.len(),
            problems.join("\n  ")
        ))
    }
}

/// What is wrong with `found` against `baseline`, the text of [`BASELINE`]: each copy it does
/// not list, and each entry no copy found is.
fn against(found: &[Copied], baseline: &str) -> Vec<String> {
    let listed: BTreeSet<&str> = baseline
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let entries: BTreeSet<String> = found.iter().map(Copied::entry).collect();
    let mut problems: Vec<String> = found
        .iter()
        .filter(|copy| !listed.contains(copy.entry().as_str()))
        .map(|copy| {
            format!(
                "{}: {} lines of code copied; share the first instead (to let a copy that was \
                 already there stand, as when a change edits or moves it, its line in {BASELINE} \
                 is `{}`)",
                copy.places(),
                copy.lines,
                copy.entry()
            )
        })
        .collect();
    problems.extend(
        listed
            .iter()
            .filter(|entry| !entries.contains(**entry))
            .map(|entry| format!("{BASELINE}: `{entry}` is no longer found; take its line out")),
    );
    problems
}

/// Whether the check reads `path`: Rust under `crates/` and `xtask/`, TypeScript under
/// `web/`, generated code aside.
fn scanned(path: &str) -> bool {
    if path.contains("/generated/") {
        return false;
    }
    let extension = std::path::Path::new(path).extension();
    if path.starts_with("web/") {
        extension.is_some_and(|extension| extension == "ts")
    } else {
        extension.is_some_and(|extension| extension == "rs")
    }
}

#[cfg(test)]
mod tests;
