//! `edit`: replace one exact piece of a file the model has read.

mod matcher;

use std::fmt::Write as _;

use glave_ports::computer::{Bytes, EntryKind};
use glave_ports::digest;
use serde::Deserialize;

use super::ToolOutcome;
use super::workspace::Workspace;
use matcher::{EditError, Edited};

/// Lines of context shown around an edit, so the model can check it without reading again.
const CONTEXT_LINES: usize = 3;
const BOM: &str = "\u{FEFF}";

#[derive(Debug, Deserialize)]
pub(crate) struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

pub(crate) async fn edit(workspace: &Workspace, args: EditArgs) -> ToolOutcome {
    let stat = match workspace.stat_for_change(&args.path).await {
        Ok(stat) => stat,
        Err(message) => return ToolOutcome::error(message),
    };
    let shown = stat.path;
    let not_found = || {
        ToolOutcome::error(format!(
            "File not found: {shown}. To create a file, use write."
        ))
    };
    if stat.kind.is_none_or(|kind| kind == EntryKind::Folder) {
        return not_found();
    }
    let Ok(bytes) = workspace.read(&shown).await.map(Bytes::into_vec) else {
        return not_found();
    };
    if let Err(message) = workspace.check_fresh(&shown, Some(&digest(&bytes))) {
        return ToolOutcome::error(message);
    }
    let Ok(original) = String::from_utf8(bytes) else {
        return ToolOutcome::error(format!("{shown} is not UTF-8 text."));
    };
    let bom = original.starts_with(BOM);
    let crlf = original.contains("\r\n");
    let text = original
        .strip_prefix(BOM)
        .unwrap_or(&original)
        .replace("\r\n", "\n");
    let old = args.old_string.replace("\r\n", "\n");
    let new = args.new_string.replace("\r\n", "\n");
    let edited = match matcher::apply(&text, &old, &new, args.replace_all) {
        Ok(edited) => edited,
        Err(error) => return ToolOutcome::error(explain(&error, &shown)),
    };
    let mut written = if crlf {
        edited.text.replace('\n', "\r\n")
    } else {
        edited.text.clone()
    };
    if bom {
        written.insert_str(0, BOM);
    }
    if let Err(message) = workspace.write(&shown, written.as_bytes()).await {
        return ToolOutcome::error(message);
    }
    workspace.remember(&shown, written.as_bytes());
    ToolOutcome::ok(success(&edited, &shown))
}

fn explain(error: &EditError, shown: &str) -> String {
    match error {
        EditError::Empty => {
            "old_string is empty. To create or replace a whole file, use write.".into()
        }
        EditError::Identical => {
            "old_string and new_string are identical; there is nothing to change.".into()
        }
        EditError::NotFound { hint } => {
            let mut message = format!(
                "old_string was not found in {shown}. It must match the file exactly, \
                 indentation included; copy it from read output without the line-number prefix."
            );
            if let Some((line, text)) = hint {
                let _ = write!(message, "\nClosest line {line}: {text}");
            }
            message
        }
        EditError::Ambiguous { lines } => {
            let lines: Vec<String> = lines.iter().map(ToString::to_string).collect();
            format!(
                "old_string matches {} places in {shown} (lines {}). Add surrounding lines to \
                 make it unique, or set replace_all to true.",
                lines.len(),
                lines.join(", ")
            )
        }
    }
}

fn success(edited: &Edited, shown: &str) -> String {
    let plural = if edited.replacements == 1 { "" } else { "s" };
    let range = if edited.first_line == edited.last_line {
        format!("line {}", edited.first_line)
    } else {
        format!("lines {}-{}", edited.first_line, edited.last_line)
    };
    let mut message = format!(
        "Edited {shown}: {} replacement{plural} at {range}{}.",
        edited.replacements,
        edited.stage.note()
    );
    let from = edited.first_line.saturating_sub(CONTEXT_LINES).max(1);
    let to = edited.last_line.saturating_add(CONTEXT_LINES);
    for (number, line) in edited
        .text
        .lines()
        .enumerate()
        .map(|(i, l)| (i.saturating_add(1), l))
    {
        if (from..=to).contains(&number) {
            let _ = write!(message, "\n{number}\t{line}");
        }
    }
    message
}
