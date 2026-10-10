//! `search`: lines matching a pattern across the project, like ripgrep. The computer searches
//! next to the files: ignored, binary and large files are skipped, and files that usually
//! hold secrets are never opened.

use std::fmt::Write as _;

use glave_ports::{Search, Searched};
use serde::Deserialize;

use super::ToolOutcome;
use super::workspace::Workspace;

/// Most matching lines returned by one search.
const MAX_MATCHES: usize = 100;
/// Longest matching line shown before it is cut.
const MAX_LINE_CHARS: usize = 200;
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
    let search = Search {
        pattern: args.pattern.clone(),
        path: args.path.clone().unwrap_or_else(|| ".".into()),
        glob: args.glob.clone(),
        ignore_case: args.ignore_case.unwrap_or(false),
        limit: MAX_MATCHES,
        line_chars: MAX_LINE_CHARS,
    };
    match workspace.search(&search).await {
        Ok(found) => ToolOutcome::ok(render(&found, &args)),
        Err(message) => ToolOutcome::error(message),
    }
}

fn render(found: &Searched, args: &SearchArgs) -> String {
    let place = &found.path;
    let mut out = String::new();
    if found.as_text {
        out.push_str("(Not a valid regular expression, so it was searched as plain text.)\n");
    }
    if found.matches.is_empty() {
        let _ = write!(out, "No matches for {:?} in {place}", args.pattern);
        if let Some(glob) = &args.glob {
            let _ = write!(out, " (files matching {glob})");
        }
        out.push('.');
    } else {
        let lines: Vec<String> = found
            .matches
            .iter()
            .map(|found| format!("{}:{}:{}", found.path, found.line, found.text))
            .collect();
        out.push_str(&lines.join("\n"));
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
