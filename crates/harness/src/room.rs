//! Making a conversation fit the model: each tool output fits a share of its context window,
//! the oldest tool outputs give way to a short note, and a refusal that states a model's real
//! context window is believed for that model for the rest of the session.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use glave_event::Part;
use glave_ports::{Message, ToolOutput};

use crate::tools::Captured;

/// Bytes per token when sizing what fits: generous, as the gateway's estimate of what fits is.
const BYTES_PER_TOKEN: u64 = 4;

/// The most bytes one tool output may give a model whose window is `window_tokens`: a quarter of
/// the window, as a summary may take, so a model with a small window still has room to read a
/// file in parts. A large window keeps each tool's own limit, which is smaller.
pub(crate) fn tool_room(window_tokens: u64) -> usize {
    usize::try_from(window_tokens / 4 * BYTES_PER_TOKEN).unwrap_or(usize::MAX)
}

/// `output` with its texts cut to `room` bytes together, each keeping its start and its end;
/// a text that finds no room left is only the note. Files are fit apart (`files::fit`).
pub(crate) fn fit(mut output: ToolOutput, room: usize) -> ToolOutput {
    let mut left = room;
    for part in &mut output.parts {
        if let Part::Text { text } = part {
            if text.len() > left {
                *text = Captured::fit(text, left);
            }
            left = left.saturating_sub(text.len());
        }
    }
    output
}

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

/// The most models whose window is remembered; the one learned longest ago is let go first.
const MAX_LEARNED: usize = 64;

/// The context windows too-long refusals have stated, by model: one model's refusal says
/// nothing of another's.
#[derive(Debug, Default)]
pub(crate) struct LearnedWindows {
    windows: Mutex<VecDeque<(String, u64)>>,
}

impl LearnedWindows {
    /// The window a refusal stated for `model`, if one has.
    pub(crate) fn of(&self, model: &str) -> Option<u64> {
        self.lock()
            .iter()
            .find(|(learned, _)| learned == model)
            .map(|(_, tokens)| *tokens)
    }

    /// Believes `tokens` for `model` from now on.
    pub(crate) fn learn(&self, model: &str, tokens: u64) {
        let mut windows = self.lock();
        windows.retain(|(learned, _)| learned != model);
        if windows.len() >= MAX_LEARNED {
            windows.pop_front();
        }
        windows.push_back((model.to_owned(), tokens));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(String, u64)>> {
        self.windows.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
    fn a_window_is_learned_for_its_model_alone() {
        let windows = LearnedWindows::default();
        windows.learn("a", 4_096);
        assert_eq!(windows.of("a"), Some(4_096));
        assert_eq!(windows.of("b"), None, "another model's window is unchanged");
        windows.learn("a", 8_192);
        assert_eq!(
            windows.of("a"),
            Some(8_192),
            "the latest refusal is believed"
        );
    }

    #[test]
    fn the_window_learned_longest_ago_goes_first() {
        let windows = LearnedWindows::default();
        for n in 0..=MAX_LEARNED {
            windows.learn(&format!("model-{n}"), 1_000);
        }
        assert_eq!(windows.of("model-0"), None);
        assert_eq!(windows.of(&format!("model-{MAX_LEARNED}")), Some(1_000));
        assert_eq!(windows.lock().len(), MAX_LEARNED);
    }

    #[test]
    fn a_tool_output_fits_a_quarter_of_the_window() {
        assert_eq!(tool_room(4_096), 4_096);
        assert_eq!(tool_room(128_000), 128_000);
        let long = ToolOutput::text(false, "x".repeat(10_000));
        let ToolOutput { parts, .. } = fit(long, tool_room(4_096));
        let [Part::Text { text }] = parts.as_slice() else {
            panic!("one text");
        };
        assert!(text.len() <= 4_096, "{}", text.len());
        assert!(text.contains("bytes of output omitted"));
        let short = ToolOutput::text(false, "kept");
        assert_eq!(fit(short.clone(), 10), short);
        let mut two = ToolOutput::text(false, "x".repeat(3_000));
        two.parts.push(Part::Text {
            text: "y".repeat(3_000),
        });
        let together: usize = fit(two, 4_096)
            .parts
            .iter()
            .map(|part| match part {
                Part::Text { text } => text.len(),
                _ => 0,
            })
            .sum();
        assert!(together <= 4_096, "texts share the room: {together}");
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
