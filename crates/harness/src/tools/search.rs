//! `search`: lines matching a pattern across the project, like ripgrep. Ignored, binary and
//! large files are skipped, and files that usually hold secrets are never opened.

use std::fmt::Write as _;
use std::path::Path;

use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use super::ToolOutcome;
use super::read::looks_binary;
use super::walk::{self, Pattern, shown};
use super::workspace::Workspace;
use crate::secrets::is_secret_file;

/// Most matching lines returned by one search.
const MAX_MATCHES: usize = 100;
/// Longest matching line shown before it is cut.
const MAX_LINE_CHARS: usize = 200;
/// Largest file searched; bigger files are usually data or build output.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Most skipped secret files named in the result.
const MAX_SECRETS_NAMED: usize = 5;

#[derive(Debug, Deserialize)]
pub(crate) struct SearchArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    ignore_case: Option<bool>,
}

pub(crate) async fn search(workspace: &Workspace, args: SearchArgs) -> ToolOutcome {
    if args.pattern.is_empty() {
        return ToolOutcome::error("The pattern is empty.");
    }
    let start = match workspace.resolve(args.path.as_deref().unwrap_or(".")).await {
        Ok(path) => path,
        Err(message) => return ToolOutcome::error(message),
    };
    let filter = match args.glob.as_deref().map(Pattern::new).transpose() {
        Ok(filter) => filter,
        Err(message) => return ToolOutcome::error(message),
    };
    let (regex, as_text) = match compile(&args.pattern, args.ignore_case.unwrap_or(false)) {
        Ok(compiled) => compiled,
        Err(message) => return ToolOutcome::error(message),
    };
    let root = workspace.root().to_owned();
    let place = shown(&root, &start);
    let scanned =
        tokio::task::spawn_blocking(move || scan(&root, &start, &regex, filter.as_ref())).await;
    let Ok(found) = scanned else {
        return ToolOutcome::error("The search stopped unexpectedly.");
    };
    ToolOutcome::ok(render(&found, &args, &place, as_text))
}

/// The pattern as a regular expression, or as plain text when it is not a valid one (`foo(`).
/// The flag says which.
fn compile(pattern: &str, ignore_case: bool) -> Result<(Regex, bool), String> {
    let build = |source: &str| {
        RegexBuilder::new(source)
            .case_insensitive(ignore_case)
            .build()
    };
    match build(pattern) {
        Ok(regex) => Ok((regex, false)),
        Err(_) => build(&regex::escape(pattern))
            .map(|regex| (regex, true))
            .map_err(|e| format!("The pattern could not be searched: {e}")),
    }
}

#[derive(Debug, Default)]
struct Found {
    lines: Vec<String>,
    /// More lines matched than were kept.
    more: bool,
    /// Files skipped because they usually hold secrets.
    secrets: Vec<String>,
}

fn scan(root: &Path, start: &Path, regex: &Regex, filter: Option<&Pattern>) -> Found {
    let mut found = Found::default();
    for path in walk::files(start) {
        let relative = path.strip_prefix(start).unwrap_or(&path);
        if filter.is_some_and(|filter| !filter.matches(relative)) {
            continue;
        }
        if is_secret_file(&path) {
            found.secrets.push(shown(root, &path));
            continue;
        }
        let Some(text) = searchable(&path) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if !regex.is_match(line) {
                continue;
            }
            if found.lines.len() == MAX_MATCHES {
                found.more = true;
                return found;
            }
            let number = index.saturating_add(1);
            found
                .lines
                .push(format!("{}:{number}:{}", shown(root, &path), clip(line)));
        }
    }
    found
}

/// The file's text, unless it is too big, unreadable or binary.
fn searchable(path: &Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if looks_binary(&bytes) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn clip(line: &str) -> String {
    let line = line.trim_end_matches('\r');
    if line.chars().count() <= MAX_LINE_CHARS {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(MAX_LINE_CHARS).collect();
    cut.push('…');
    cut
}

fn render(found: &Found, args: &SearchArgs, place: &str, as_text: bool) -> String {
    let mut out = String::new();
    if as_text {
        out.push_str("(Not a valid regular expression, so it was searched as plain text.)\n");
    }
    if found.lines.is_empty() {
        let _ = write!(out, "No matches for {:?} in {place}", args.pattern);
        if let Some(glob) = &args.glob {
            let _ = write!(out, " (files matching {glob})");
        }
        out.push('.');
    } else {
        out.push_str(&found.lines.join("\n"));
    }
    if found.more {
        let _ = write!(
            out,
            "\n[Stopped at {MAX_MATCHES} matches. Narrow the pattern, path or glob to see the rest.]"
        );
    }
    if !found.secrets.is_empty() {
        let named: Vec<&str> = found
            .secrets
            .iter()
            .take(MAX_SECRETS_NAMED)
            .map(String::as_str)
            .collect();
        let _ = write!(
            out,
            "\n[Not searched, since they usually hold secrets: {}{}]",
            named.join(", "),
            if found.secrets.len() > MAX_SECRETS_NAMED {
                ", …"
            } else {
                ""
            }
        );
    }
    out
}

#[cfg(test)]
mod tests;
