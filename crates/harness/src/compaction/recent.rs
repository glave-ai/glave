//! The latest tool results, kept word for word through a compaction, so the conversation that
//! restarts from a summary still holds the files it was just reading and needn't read them again.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glave_event::{CallId, Part};
use glave_ports::{Message, ToolCall};

use super::arguments_text;
use crate::room::OMITTED;

/// The latest tool calls in `messages` with what they returned, as text, oldest first: as many
/// of the newest as fit in `room` characters together, each kept whole or not at all. `None`
/// when not even the newest fits.
pub(crate) fn recent(messages: &[Message], room: usize) -> Option<String> {
    let calls: BTreeMap<&CallId, &ToolCall> = messages
        .iter()
        .flat_map(|message| match message {
            Message::Assistant { tool_calls, .. } => tool_calls.as_slice(),
            Message::User { .. } | Message::Tool { .. } => &[],
        })
        .map(|call| (&call.call_id, call))
        .collect();
    let mut kept = Vec::new();
    let mut left = room;
    for message in messages.iter().rev() {
        let Message::Tool {
            call_id, output, ..
        } = message
        else {
            continue;
        };
        let Some(call) = calls.get(call_id) else {
            continue;
        };
        let Some(entry) = entry(call, output) else {
            continue;
        };
        if entry.len() > left {
            break;
        }
        left = left.saturating_sub(entry.len());
        kept.push(entry);
    }
    if kept.is_empty() {
        return None;
    }
    kept.reverse();
    Some(kept.concat())
}

/// One call and its output, or `None` when its output was already removed to fit.
fn entry(call: &ToolCall, output: &[Part]) -> Option<String> {
    let text: Vec<&str> = output
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            Part::Reasoning { .. } | Part::File { .. } => None,
        })
        .collect();
    let text = text.join("\n");
    if text == OMITTED {
        return None;
    }
    let mut entry = String::new();
    let _ = write!(
        entry,
        "\n\n`{}` {}:\n{}",
        call.tool,
        arguments_text(&call.arguments),
        text.trim_end()
    );
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::Arguments;

    fn read(id: &str, path: &str, output: &str) -> [Message; 2] {
        let mut map = serde_json::Map::new();
        map.insert("path".into(), path.into());
        [
            Message::Assistant {
                parts: Vec::new(),
                tool_calls: vec![ToolCall {
                    call_id: CallId::new(id).unwrap(),
                    tool: "read".into(),
                    arguments: Arguments::Json(map),
                }],
            },
            Message::Tool {
                call_id: CallId::new(id).unwrap(),
                is_error: false,
                output: vec![Part::Text {
                    text: output.into(),
                }],
            },
        ]
    }

    #[test]
    fn the_newest_results_that_fit_are_kept_whole_oldest_first() {
        let messages: Vec<Message> = [
            read("1", "a.py", &"a".repeat(100)),
            read("2", "b.py", "bb"),
            read("3", "c.py", "cc"),
        ]
        .concat();
        let kept = recent(&messages, 80).unwrap();
        assert_eq!(
            kept,
            "\n\n`read` {\"path\":\"b.py\"}:\nbb\n\n`read` {\"path\":\"c.py\"}:\ncc"
        );
        assert!(
            recent(&messages, 10).is_none(),
            "the newest alone is too big"
        );
    }

    #[test]
    fn outputs_already_removed_are_not_kept() {
        let messages: Vec<Message> = read("1", "a.py", OMITTED).into();
        assert!(recent(&messages, 1_000).is_none());
    }
}
