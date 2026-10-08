//! `read`: a file with numbered lines, or a folder's entries apart from what git ignores.

use std::fmt::Write as _;

use serde::Deserialize;

use super::ToolOutcome;
use super::walk;
use super::workspace::Workspace;

/// Most lines returned by one read.
const MAX_LINES: usize = 2_000;
/// Most bytes of text returned by one read.
const MAX_BYTES: usize = 50 * 1024;
/// Longest line returned before it is cut.
const MAX_LINE_CHARS: usize = 2_000;
/// Largest file read at all; bigger files are for `bash` with `sed -n` or `rg`.
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Most folder entries listed.
const MAX_ENTRIES: usize = 1_000;

#[derive(Debug, Deserialize)]
pub(crate) struct ReadArgs {
    path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

pub(crate) async fn read(workspace: &Workspace, args: ReadArgs) -> ToolOutcome {
    let path = match workspace.resolve(&args.path).await {
        Ok(path) => path,
        Err(message) => return ToolOutcome::error(message),
    };
    let shown = workspace.show(&path);
    let Ok(metadata) = tokio::fs::metadata(&path).await else {
        return ToolOutcome::error(format!("File not found: {shown}"));
    };
    if metadata.is_dir() {
        return list(&path, &shown).await;
    }
    if metadata.len() > MAX_FILE_BYTES {
        return ToolOutcome::error(format!(
            "{shown} is {} bytes, too large to read. Use bash with sed -n or rg to look inside it.",
            metadata.len()
        ));
    }
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) => return ToolOutcome::error(format!("Could not read {shown}: {e}")),
    };
    if looks_binary(&bytes) {
        return ToolOutcome::error(format!("{shown} is a binary file."));
    }
    workspace.remember(&path, &bytes);
    numbered(&String::from_utf8_lossy(&bytes), args.offset, args.limit)
}

fn numbered(text: &str, offset: Option<usize>, limit: Option<usize>) -> ToolOutcome {
    let total = text.lines().count();
    let first = offset.unwrap_or(1).max(1);
    if total == 0 {
        return ToolOutcome::ok("(empty file)");
    }
    if first > total {
        return ToolOutcome::error(format!(
            "Offset {first} is past the end of the file ({total} lines)."
        ));
    }
    let limit = limit.unwrap_or(MAX_LINES).clamp(1, MAX_LINES);
    let mut out = String::new();
    let mut last = first.saturating_sub(1);
    for (number, line) in text
        .lines()
        .enumerate()
        .skip(first.saturating_sub(1))
        .take(limit)
    {
        let number = number.saturating_add(1);
        let line = cut_line(line);
        if out.len().saturating_add(line.len()) > MAX_BYTES && last >= first {
            break;
        }
        let _ = writeln!(out, "{number}\t{line}");
        last = number;
    }
    if last < total {
        let _ = write!(
            out,
            "(lines {first}-{last} of {total}; use offset={} to continue)",
            last.saturating_add(1)
        );
    } else {
        let _ = write!(out, "(end of file, {total} lines)");
    }
    ToolOutcome::ok(out)
}

fn cut_line(line: &str) -> String {
    if line.chars().count() <= MAX_LINE_CHARS {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(MAX_LINE_CHARS).collect();
    cut.push_str("… [line truncated]");
    cut
}

/// A NUL byte, or more than 30% control bytes, in the first 4 KiB.
pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    let head = bytes.get(..4_096).unwrap_or(bytes);
    if head.contains(&0) {
        return true;
    }
    let control = head
        .iter()
        .filter(|b| **b < 0x09 || (0x0E..0x20).contains(*b))
        .count();
    control.saturating_mul(10) > head.len().saturating_mul(3)
}

async fn list(path: &std::path::Path, shown: &str) -> ToolOutcome {
    let dir = path.to_owned();
    let (names, left_out) = match tokio::task::spawn_blocking(move || walk::entries(&dir)).await {
        Ok(Ok(listed)) => listed,
        Ok(Err(e)) => return ToolOutcome::error(format!("Could not list {shown}: {e}")),
        Err(_) => return ToolOutcome::error(format!("Could not list {shown}.")),
    };
    let total = names.len();
    let mut lines: Vec<String> = names.into_iter().take(MAX_ENTRIES).collect();
    if total > MAX_ENTRIES {
        lines.push(format!("({MAX_ENTRIES} of {total} entries shown)"));
    }
    if left_out > 0 {
        lines.push(format!(
            "({left_out} more that git ignores, such as build output; read them by name if needed)"
        ));
    }
    if lines.is_empty() {
        return ToolOutcome::ok("(empty folder)");
    }
    ToolOutcome::ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_numbered_with_a_footer() {
        let out = numbered("alpha\nbeta\ngamma\n", None, None);
        assert!(!out.is_error);
        assert_eq!(
            out.text,
            "1\talpha\n2\tbeta\n3\tgamma\n(end of file, 3 lines)"
        );
    }

    #[test]
    fn offset_and_limit_page_through() {
        let out = numbered("a\nb\nc\nd\n", Some(2), Some(2));
        assert_eq!(
            out.text,
            "2\tb\n3\tc\n(lines 2-3 of 4; use offset=4 to continue)"
        );
        let past = numbered("a\nb\n", Some(9), None);
        assert!(past.is_error);
        assert!(past.text.contains("past the end"));
    }

    #[test]
    fn long_lines_are_cut() {
        let long = "x".repeat(MAX_LINE_CHARS + 10);
        assert!(
            numbered(&long, None, None)
                .text
                .contains("[line truncated]")
        );
    }

    #[test]
    fn binary_is_detected() {
        assert!(looks_binary(b"\x7fELF\x00\x01"));
        assert!(!looks_binary("hello, wörld\n\ttabbed\r\n".as_bytes()));
    }
}
