//! Making a conversation fit the model: the oldest tool outputs give way to a short note, and a
//! refusal that states the model's real context window is believed for the rest of the session.

use glave_event::Part;
use glave_ports::Message;

/// What a removed tool output becomes.
pub(crate) const OMITTED: &str = "[This output was removed to fit the context window.]";

/// Replaces the oldest half of the tool outputs still there (at least one) with a note. False
/// when none was left to remove.
pub(crate) fn trim_oldest(messages: &mut [Message]) -> bool {
    let kept: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            Message::Tool { output, .. } if !omitted(output) => Some(index),
            _ => None,
        })
        .collect();
    if kept.is_empty() {
        return false;
    }
    for index in kept.iter().take(kept.len().div_ceil(2)) {
        if let Some(Message::Tool { output, .. }) = messages.get_mut(*index) {
            *output = vec![Part::Text {
                text: OMITTED.to_owned(),
            }];
        }
    }
    true
}

fn omitted(output: &[Part]) -> bool {
    matches!(output, [Part::Text { text }] if text == OMITTED)
}

/// What servers say just before the window's size, lowercase: OpenAI, vLLM and OpenRouter
/// ("maximum context length is 32768 tokens"), LM Studio ("context length of only 4096"),
/// llama.cpp ("available context size (8192 tokens)") and Anthropic ("210000 tokens > 200000").
const BEFORE_WINDOW: &[&str] = &[
    "maximum context length is",
    "context length of only",
    "context window of",
    "available context size (",
    "tokens >",
];

/// The context window a too-long refusal states, in tokens, if it states one.
pub(crate) fn stated_window(message: &str) -> Option<u64> {
    let message = message.to_ascii_lowercase();
    BEFORE_WINDOW.iter().find_map(|phrase| {
        let start = message.find(phrase)?.checked_add(phrase.len())?;
        let digits: String = message
            .get(start..)?
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ',')
            .filter(char::is_ascii_digit)
            .collect();
        digits.parse().ok().filter(|tokens| *tokens > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_event::CallId;

    #[test]
    fn the_window_is_read_from_what_servers_say() {
        let said = [
            (
                "This model's maximum context length is 32,768 tokens. However, you requested 40000",
                32_768,
            ),
            (
                "Trying to keep the first 5000 tokens when context the overflows. However, the model is loaded with context length of only 4096 tokens",
                4_096,
            ),
            (
                "the request exceeds the available context size (8192 tokens), try increasing it",
                8_192,
            ),
            (
                "prompt is too long: 210000 tokens > 200000 maximum",
                200_000,
            ),
        ];
        for (message, tokens) in said {
            assert_eq!(stated_window(message), Some(tokens), "{message}");
        }
        assert_eq!(stated_window("the prompt is too long"), None);
    }

    fn tool(text: &str) -> Message {
        Message::Tool {
            call_id: CallId::new("call_1").unwrap(),
            is_error: false,
            output: vec![Part::Text { text: text.into() }],
        }
    }

    fn texts(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::Tool { output, .. } => match output.as_slice() {
                    [Part::Text { text }] => Some(text.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_oldest_half_of_the_outputs_go_first_until_none_is_left() {
        let mut messages = vec![tool("a"), tool("b"), tool("c")];
        assert!(trim_oldest(&mut messages));
        assert_eq!(texts(&messages), [OMITTED, OMITTED, "c"]);
        assert!(trim_oldest(&mut messages));
        assert_eq!(texts(&messages), [OMITTED, OMITTED, OMITTED]);
        assert!(!trim_oldest(&mut messages));
    }
}
