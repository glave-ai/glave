//! AGENTS.md in a project's subfolders, sent when the work reaches them, as Claude Code sends a
//! subfolder's: when `read`, `write` or `edit` touches a path below the workspace's root, each
//! AGENTS.md from the first folder below the root down to the path's own folder follows the
//! tool's result, labelled with its folder, unless the conversation holds it already. What the
//! conversation holds is what was sent, so a resumed thread knows it; a compacted thread, or one
//! whose outputs gave way to fit the window, is sent it again when the work next reaches there.
//! Together with what the thread opened with, they stay within [`MAX_BYTES`].

use glave_event::Part;
use glave_ports::computer::EntryKind;
use glave_ports::{BlobStore, EventLog, Message, ModelProvider, ToolCall, ToolOutput};

use super::{AGENTS_MD, CUT_SHORT, HEADING, MAX_BYTES, within};
use crate::approval::Approver;
use crate::harness::{Harness, Thread};
use crate::tools::Tool;

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// `output` of the built-in `call` on `thread`, followed by the AGENTS.md of each
    /// subfolder it reached that the model does not see now. A file that cannot be read is
    /// left out: the call's own result is what matters.
    pub(crate) async fn with_nested_instructions(
        &self,
        thread: &Thread,
        call: &ToolCall,
        mut output: ToolOutput,
    ) -> ToolOutput {
        let path = Tool::touches(&call.tool, &call.arguments);
        let Some(path) = path.filter(|_| !output.is_error) else {
            return output;
        };
        let workspace = self.workspace_for(thread);
        let Ok(stat) = workspace.stat(path).await else {
            return output;
        };
        let opened = thread.instructions.as_ref().map_or(0, String::len);
        let mut room = MAX_BYTES
            .saturating_sub(opened)
            .saturating_sub(sent_bytes(&thread.messages));
        let is_folder = stat.kind == Some(EntryKind::Folder);
        for folder in folders(&stat.path, is_folder) {
            let heading = format!("{HEADING}{folder}\n");
            let file = format!("{folder}/{AGENTS_MD}");
            if room == 0 || file == stat.path || holds(&thread.messages, &heading) {
                continue;
            }
            let Ok(bytes) = workspace.read(&file).await else {
                continue;
            };
            let bytes = bytes.into_vec();
            let text = String::from_utf8_lossy(&bytes);
            let (text, cut) = within(text.trim(), room);
            if text.is_empty() {
                continue;
            }
            room = room.saturating_sub(text.len());
            let note = if cut { CUT_SHORT } else { "" };
            output.parts.push(Part::Text {
                text: format!(
                    "{heading}\n<INSTRUCTIONS>\nFrom {file}:\n{text}{note}\n</INSTRUCTIONS>"
                ),
            });
        }
        output
    }
}

/// The folders below the root whose AGENTS.md apply to `path`, a path inside the workspace
/// relative to its root: each from the first below the root down to the folder `path` is in,
/// or `path` itself when it is a folder. None for a path outside the workspace.
fn folders(path: &str, is_folder: bool) -> Vec<String> {
    let mut names: Vec<&str> = path
        .split('/')
        .filter(|name| !name.is_empty() && *name != ".")
        .collect();
    if names.contains(&"..") {
        return Vec::new();
    }
    if !is_folder {
        names.pop();
    }
    (1..=names.len())
        .filter_map(|depth| names.get(..depth))
        .map(|names| names.join("/"))
        .collect()
}

/// The subfolders' instructions the conversation holds now: parts of tool results after the
/// first, where only glave puts them, so an output that opens like them is not taken for them.
fn sent(messages: &[Message]) -> impl Iterator<Item = &str> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::Tool { output, .. } => output.get(1..),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            Part::Text { text } if text.starts_with(HEADING) => Some(text.as_str()),
            _ => None,
        })
}

/// Whether the conversation holds the instructions that open with `heading`.
fn holds(messages: &[Message], heading: &str) -> bool {
    sent(messages).any(|text| text.starts_with(heading))
}

/// How many bytes of subfolders' instructions the conversation holds now.
fn sent_bytes(messages: &[Message]) -> usize {
    sent(messages).map(str::len).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::CallId;

    const NONE: [&str; 0] = [];

    #[test]
    fn a_path_reaches_each_folder_below_the_root_outermost_first() {
        assert_eq!(
            folders("packages/app/src/main.rs", false),
            ["packages", "packages/app", "packages/app/src"]
        );
        assert_eq!(folders("packages/app", true), ["packages", "packages/app"]);
        assert_eq!(
            folders("main.rs", false),
            NONE,
            "the root's are sent already"
        );
        assert_eq!(folders(".", true), NONE);
        assert_eq!(folders("../elsewhere/file", false), NONE);
    }

    #[test]
    fn what_was_sent_is_what_the_conversation_holds() {
        let sent = format!("{HEADING}packages/app\n\n<INSTRUCTIONS>\nFrom …");
        let messages = vec![Message::Tool {
            call_id: CallId::new("call_1").unwrap(),
            is_error: false,
            output: vec![
                Part::Text {
                    text: "1\tfn main() {}".into(),
                },
                Part::Text { text: sent.clone() },
            ],
        }];
        assert!(holds(&messages, &format!("{HEADING}packages/app\n")));
        let mimic = vec![Message::Tool {
            call_id: CallId::new("call_2").unwrap(),
            is_error: false,
            output: vec![Part::Text { text: sent.clone() }],
        }];
        assert!(
            !holds(&mimic, &format!("{HEADING}packages/app\n")),
            "an output that only looks like them is not them"
        );
        assert!(!holds(&messages, &format!("{HEADING}packages\n")));
        assert_eq!(sent_bytes(&messages), sent.len());
    }
}
