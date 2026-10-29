//! `read`: a file with numbered lines, or a folder's entries apart from what git ignores. An
//! image or a PDF is handed to the model as a person's is.

use std::fmt::Write as _;

use glave_ports::computer::{EntryKind, Listing, looks_binary};
use glave_ports::{PDF, ToolFile};
use serde::Deserialize;

use super::ToolOutcome;
use super::workspace::Workspace;
use crate::files::{MAX_FILE, sniffed};

/// Most lines returned by one read.
const MAX_LINES: usize = 2_000;
/// Most bytes of text returned by one read, however much room the model has.
const MAX_BYTES: usize = 50 * 1024;
/// Room kept for the line that says which lines were read and where to go on.
const FOOTER_BYTES: usize = 64;
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

/// Reads what `args` names, its text within `room` bytes.
pub(crate) async fn read(workspace: &Workspace, args: ReadArgs, room: usize) -> ToolOutcome {
    let stat = match workspace.stat(&args.path).await {
        Ok(stat) => stat,
        Err(message) => return ToolOutcome::error(message),
    };
    let shown = stat.path;
    match stat.kind {
        None => return ToolOutcome::error(format!("File not found: {shown}")),
        Some(EntryKind::Folder) => return list(workspace, &shown).await,
        Some(EntryKind::File | EntryKind::Link) => {}
    }
    if stat.size > MAX_FILE_BYTES {
        return ToolOutcome::error(format!(
            "{shown} is {} bytes, too large to read. Use bash with sed -n or rg to look inside it.",
            stat.size
        ));
    }
    let bytes = match workspace.read(&shown).await {
        Ok(bytes) => bytes.into_vec(),
        Err(e) => return ToolOutcome::error(format!("Could not read {shown}: {}", e.message())),
    };
    if let Some(media_type) = sniffed(&bytes) {
        return handed(&shown, media_type, bytes);
    }
    if looks_binary(&bytes) {
        return ToolOutcome::error(format!("{shown} is a binary file."));
    }
    workspace.remember(&shown, &bytes);
    numbered(
        &String::from_utf8_lossy(&bytes),
        args.offset,
        args.limit,
        room,
    )
}

/// The image or PDF at `shown`, `bytes` of `media_type`, handed to the model after a line
/// saying what it is; the harness keeps it as it keeps a person's, an image fit to send.
fn handed(shown: &str, media_type: &str, bytes: Vec<u8>) -> ToolOutcome {
    let name = shown.rsplit('/').next().unwrap_or(shown).to_owned();
    let size = bytes.len();
    if media_type == PDF {
        if size as u64 > MAX_FILE {
            return ToolOutcome::error(format!(
                "{shown} is a PDF of {size} bytes, too large to read whole (at most {MAX_FILE}). \
                 Use bash to look inside it, such as with pdftotext."
            ));
        }
        let file = ToolFile {
            name,
            media_type: PDF.to_owned(),
            bytes,
        };
        return ToolOutcome {
            file: Some(file),
            ..ToolOutcome::ok(format!("{shown} is a PDF of {size} bytes."))
        };
    }
    let file = ToolFile {
        name,
        media_type: media_type.to_owned(),
        bytes,
    };
    ToolOutcome {
        file: Some(file),
        ..ToolOutcome::ok(format!("{shown} is an image of {size} bytes."))
    }
}

fn numbered(text: &str, offset: Option<usize>, limit: Option<usize>, room: usize) -> ToolOutcome {
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
    // The line after the page fits too.
    let most = MAX_BYTES.min(room.saturating_sub(FOOTER_BYTES));
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
        if out.len().saturating_add(line.len()) > most && last >= first {
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

async fn list(workspace: &Workspace, shown: &str) -> ToolOutcome {
    let listing = Listing {
        skip_ignored: true,
        ..Listing::of(shown)
    };
    let listed = match workspace.list(&listing).await {
        Ok(listed) => listed,
        Err(e) => return ToolOutcome::error(format!("Could not list {shown}: {}", e.message())),
    };
    let total = listed.entries.len();
    let mut lines: Vec<String> = listed
        .entries
        .iter()
        .take(MAX_ENTRIES)
        .map(|entry| {
            let name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
            if entry.kind == EntryKind::Folder {
                format!("{name}/")
            } else {
                name.to_owned()
            }
        })
        .collect();
    if total > MAX_ENTRIES {
        lines.push(format!("({MAX_ENTRIES} of {total} entries shown)"));
    }
    if listed.ignored > 0 {
        lines.push(format!(
            "({} more that git ignores, such as build output; read them by name if needed)",
            listed.ignored
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
        let out = numbered("alpha\nbeta\ngamma\n", None, None, usize::MAX);
        assert!(!out.is_error);
        assert_eq!(
            out.text,
            "1\talpha\n2\tbeta\n3\tgamma\n(end of file, 3 lines)"
        );
    }

    #[test]
    fn offset_and_limit_page_through() {
        let out = numbered("a\nb\nc\nd\n", Some(2), Some(2), usize::MAX);
        assert_eq!(
            out.text,
            "2\tb\n3\tc\n(lines 2-3 of 4; use offset=4 to continue)"
        );
        let past = numbered("a\nb\n", Some(9), None, usize::MAX);
        assert!(past.is_error);
        assert!(past.text.contains("past the end"));
    }

    #[test]
    fn a_small_room_reads_fewer_lines_and_says_where_to_go_on() {
        let text = "a line\n".repeat(1_000);
        let out = numbered(&text, None, None, 1_000);
        assert!(out.text.len() <= 1_100, "{}", out.text.len());
        assert!(out.text.starts_with("1\ta line\n2\t"), "{}", out.text);
        assert!(out.text.contains(" of 1000; use offset="), "{}", out.text);
    }

    #[test]
    fn long_lines_are_cut() {
        let long = "x".repeat(MAX_LINE_CHARS + 10);
        assert!(
            numbered(&long, None, None, usize::MAX)
                .text
                .contains("[line truncated]")
        );
    }

    #[test]
    fn binary_is_detected() {
        assert!(looks_binary(b"\x7fELF\x00\x01"));
        assert!(!looks_binary("hello, wörld\n\ttabbed\r\n".as_bytes()));
    }

    proptest::proptest! {
        /// Any page of any file numbers its lines consecutively from the offset asked for, and
        /// a page past the end is refused, never empty or wrong.
        #[test]
        fn pages_number_lines_consecutively(
            lines in proptest::collection::vec("[a-z ]{0,20}", 0..60),
            offset in 1_usize..80,
            limit in 1_usize..80,
        ) {
            let text: String = lines.iter().map(|line| [line.as_str(), "\n"].concat()).collect();
            let out = numbered(&text, Some(offset), Some(limit), usize::MAX);
            if lines.is_empty() {
                proptest::prop_assert_eq!(out.text, "(empty file)");
                return Ok(());
            }
            if offset > lines.len() {
                proptest::prop_assert!(out.is_error);
                return Ok(());
            }
            let numbers: Vec<usize> = out
                .text
                .lines()
                .filter_map(|line| line.split_once('\t'))
                .map(|(number, _)| number.parse().unwrap())
                .collect();
            let last = (offset + limit - 1).min(lines.len());
            proptest::prop_assert_eq!(numbers, (offset..=last).collect::<Vec<_>>());
        }
    }
}
