//! How large a conversation is: its characters, the next request's size in tokens as the
//! provider last counted it and what came since, and where its older messages end.

use glave_event::{Arguments, Part};
use glave_ports::{Message, ToolSpec};

use crate::harness::Thread;
use crate::toolset::ToolSet;

/// Characters per token when nothing better is known. Code runs closer to 3, so estimates
/// run low; the threshold leaves room for that.
pub(super) const CHARS_PER_TOKEN: u64 = 4;

/// Characters of everything the model sees in `messages`.
pub(crate) fn chars(messages: &[Message]) -> u64 {
    let total: usize = messages
        .iter()
        .map(|message| match message {
            Message::User { parts } | Message::Tool { output: parts, .. } => parts_chars(parts),
            Message::Assistant { parts, tool_calls } => {
                let calls: usize = tool_calls
                    .iter()
                    .map(|call| {
                        call.tool
                            .len()
                            .saturating_add(arguments_chars(&call.arguments))
                    })
                    .sum();
                parts_chars(parts).saturating_add(calls)
            }
        })
        .sum();
    u64::try_from(total).unwrap_or(u64::MAX)
}

fn parts_chars(parts: &[Part]) -> usize {
    parts
        .iter()
        .map(|part| match part {
            Part::Text { text } => text.len(),
            Part::Reasoning { text, .. } => text.as_ref().map_or(0, String::len),
            Part::File { .. } => 0,
        })
        .sum()
}

fn arguments_chars(arguments: &Arguments) -> usize {
    match arguments {
        Arguments::Json(map) => serde_json::to_string(map).map_or(0, |json| json.len()),
        Arguments::Malformed(raw) => raw.len(),
    }
}

/// A tool call's arguments as the model wrote them.
pub(super) fn arguments_text(arguments: &Arguments) -> String {
    match arguments {
        Arguments::Json(map) => serde_json::to_string(map).unwrap_or_default(),
        Arguments::Malformed(raw) => raw.clone(),
    }
}

/// The estimated size of the next request, in tokens: the last reported count plus what
/// was added since, or an estimate of everything when nothing was reported yet.
pub(super) fn next_request_tokens(thread: &Thread, offer: &ToolSet) -> u64 {
    let now = chars(&thread.messages);
    match thread.measured {
        Some(measured) => measured
            .tokens
            .saturating_add(now.saturating_sub(measured.chars).div_ceil(CHARS_PER_TOKEN)),
        None => now
            .saturating_add(base_chars(&offer.system, &offer.specs))
            .div_ceil(CHARS_PER_TOKEN),
    }
}

/// Characters every request carries besides the conversation: instructions and tools.
fn base_chars(system: &str, tools: &[ToolSpec]) -> u64 {
    let tools = serde_json::to_string(tools).map_or(0, |json| json.len());
    u64::try_from(system.len().saturating_add(tools)).unwrap_or(u64::MAX)
}

/// Whether `tokens` fill at least `percent` of `window`.
pub(super) fn past(tokens: u64, window: u64, percent: u8) -> bool {
    tokens.saturating_mul(100) >= window.saturating_mul(u64::from(percent))
}

/// Where the older part of `messages` ends, leaving the newest that fit in `keep` characters:
/// at the first response of the model's, at least two messages in, after which no more than
/// `keep` follow, so the newer part opens with a response and holds every call's result with
/// its call. `None` when even the latest response and what follows it hold more.
pub(super) fn older_part(messages: &[Message], keep: u64) -> Option<usize> {
    let total = chars(messages);
    let mut before = 0_u64;
    for (index, message) in messages.iter().enumerate() {
        let fits = total.saturating_sub(before) <= keep;
        if index >= 2 && fits && matches!(message, Message::Assistant { .. }) {
            return Some(index);
        }
        before = before.saturating_add(chars(std::slice::from_ref(message)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::CallId;
    use glave_ports::ToolCall;

    fn user(text: &str) -> Message {
        Message::User {
            parts: vec![Part::Text { text: text.into() }],
        }
    }

    fn call(id: &str) -> Message {
        Message::Assistant {
            parts: Vec::new(),
            tool_calls: vec![ToolCall {
                call_id: CallId::new(id).unwrap(),
                tool: "read".into(),
                arguments: Arguments::Malformed(String::new()),
            }],
        }
    }

    fn result(id: &str, text: &str) -> Message {
        Message::Tool {
            call_id: CallId::new(id).unwrap(),
            is_error: false,
            output: vec![Part::Text { text: text.into() }],
        }
    }

    #[test]
    fn characters_count_what_the_model_sees() {
        let messages = vec![
            user("abcd"),
            Message::Assistant {
                parts: vec![Part::Reasoning {
                    text: Some("xy".into()),
                    opaque: None,
                }],
                tool_calls: vec![ToolCall {
                    call_id: CallId::new("call_1").unwrap(),
                    tool: "read".into(),
                    arguments: Arguments::Malformed("{p".into()),
                }],
            },
        ];
        assert_eq!(chars(&messages), 4 + 2 + 4 + 2);
    }

    #[test]
    fn the_older_part_ends_at_the_first_response_after_which_the_rest_fits() {
        let messages = vec![
            user(&"a".repeat(10)),
            call("1"),
            result("1", &"b".repeat(50)),
            call("2"),
            result("2", &"c".repeat(40)),
            call("3"),
            result("3", &"d".repeat(10)),
        ];
        assert_eq!(
            older_part(&messages, 60),
            Some(3),
            "the second response opens the newer: 58 characters"
        );
        assert_eq!(older_part(&messages, 20), Some(5), "the latest alone");
        assert_eq!(older_part(&messages, 10), None, "not even the latest fits");
        let one_big = vec![user("hi"), call("1"), result("1", &"b".repeat(400))];
        assert_eq!(
            older_part(&one_big, 100),
            None,
            "too early to keep anything"
        );
    }
}
