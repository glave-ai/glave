//! What a run did, read back from the thread's events as `glave --events` exports them.

use std::collections::HashSet;

use glave_event::{Body, Outcome, Part, RawEvent, Role, decode};

/// The facts a task is graded on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    pub threads: usize,
    pub turns: u32,
    pub model_calls: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    /// Total cost in micros of the reported currency; `None` when any call reported none.
    pub cost_micros: Option<i64>,
    /// How each turn ended, in order.
    pub outcomes: Vec<Outcome>,
    /// The text of the last assistant message in the last turn.
    pub answer: String,
}

impl Default for Trace {
    fn default() -> Self {
        Self {
            threads: 0,
            turns: 0,
            model_calls: 0,
            tool_calls: 0,
            tool_errors: 0,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            cost_micros: Some(0),
            outcomes: Vec::new(),
            answer: String::new(),
        }
    }
}

/// Builds the trace from JSON Lines events.
pub fn parse(jsonl: &str) -> Result<Trace, String> {
    let mut trace = Trace::default();
    let mut threads = HashSet::new();
    for (index, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let bad = |err: glave_event::DecodeError| format!("event line {}: {err}", index + 1);
        let event = decode(RawEvent::from_json(line).map_err(bad)?).map_err(bad)?;
        threads.insert(event.header.thread);
        trace.add(event.body);
    }
    trace.threads = threads.len();
    Ok(trace)
}

impl Trace {
    fn add(&mut self, body: Body) {
        match body {
            Body::TurnStarted(_) => {
                self.turns = self.turns.saturating_add(1);
                self.answer.clear();
            }
            Body::MessageAdded(message) if message.role == Role::Assistant => {
                self.answer = text_of(&message.parts);
            }
            Body::ToolCallRequested(_) => self.tool_calls = self.tool_calls.saturating_add(1),
            Body::ToolCallCompleted(done) if done.is_error => {
                self.tool_errors = self.tool_errors.saturating_add(1);
            }
            Body::ModelResponded(response) => {
                self.model_calls = self.model_calls.saturating_add(1);
                let counts = response.usage.counts();
                self.input_tokens = self.input_tokens.saturating_add(counts.input);
                self.output_tokens = self.output_tokens.saturating_add(counts.output);
                self.cached_tokens = self.cached_tokens.saturating_add(counts.cache_read);
                self.cost_micros = match (self.cost_micros, response.cost) {
                    (Some(total), Some(cost)) => total.checked_add(cost.amount.micros),
                    _ => None,
                };
            }
            Body::TurnEnded(ended) => self.outcomes.push(ended.outcome),
            _ => {}
        }
    }
}

fn text_of(parts: &[Part]) -> String {
    let texts: Vec<&str> = parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    texts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The golden events: one thread, one turn, one tool call.
    const GOLDEN: [&str; 7] = [
        include_str!("../../../crates/event/fixtures/thread.started/v1.json"),
        include_str!("../../../crates/event/fixtures/turn.started/v1.json"),
        include_str!("../../../crates/event/fixtures/message.added/v1.json"),
        include_str!("../../../crates/event/fixtures/tool.call.requested/v1.json"),
        include_str!("../../../crates/event/fixtures/tool.call.completed/v1.json"),
        include_str!("../../../crates/event/fixtures/model.responded/v1.json"),
        include_str!("../../../crates/event/fixtures/turn.ended/v1.json"),
    ];

    #[test]
    fn the_golden_thread_reads_as_one_completed_turn() {
        let trace = parse(&GOLDEN.join("\n")).unwrap();
        assert_eq!(
            trace,
            Trace {
                threads: 1,
                turns: 1,
                model_calls: 1,
                tool_calls: 1,
                tool_errors: 0,
                input_tokens: 43_003,
                output_tokens: 120,
                cached_tokens: 40_000,
                cost_micros: Some(4_200),
                outcomes: vec![Outcome::Completed],
                answer: "Moved 214 files into 6 folders.".to_owned(),
            }
        );
    }

    #[test]
    fn a_bad_line_names_its_number() {
        let error = parse(&format!("{}\nnot json\n", GOLDEN[0].trim())).unwrap_err();
        assert!(error.starts_with("event line 2:"), "{error}");
    }
}
