//! What a run did, read back from the thread's events as `glave --events` exports them.

use std::collections::{HashMap, HashSet};

use glave_event::{
    Body, CallId, GoalReason, Outcome, Part, RawEvent, Role, ToolCallCompleted, decode,
};

/// The facts a task is graded on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    pub threads: usize,
    pub turns: u32,
    pub model_calls: u32,
    pub tool_calls: u32,
    pub(crate) tool_errors: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    /// Total cost in micros of the reported currency; `None` when any call reported none.
    pub cost_micros: Option<i64>,
    /// How each turn ended, in order.
    pub outcomes: Vec<Outcome>,
    /// The text of the last assistant message in the last turn.
    pub answer: String,
    /// Everything commands printed back: the output of every `bash` call.
    pub printed: String,
    /// Why the last goal ended, if one did.
    pub goal: Option<GoalReason>,
    /// How many times the conversation was restarted from a summary.
    pub compactions: u32,
    /// Tool calls that gave nothing new: the same tool with the same arguments and the same
    /// result as a call before them, as when a file is read again unchanged or a failing
    /// command is run again. A test rerun after a change gives a new result, so it is not one.
    pub repeated_steps: u32,
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
            printed: String::new(),
            goal: None,
            compactions: 0,
            repeated_steps: 0,
        }
    }
}

/// The tool calls seen so far.
#[derive(Default)]
struct Steps {
    /// Each call not yet finished: its tool and arguments.
    open: HashMap<CallId, (String, String)>,
    /// Every finished call, as its tool, arguments and result.
    done: HashSet<(String, String, String)>,
}

/// Builds the trace from JSON Lines events.
pub(crate) fn parse(jsonl: &str) -> Result<Trace, String> {
    let mut trace = Trace::default();
    let mut threads = HashSet::new();
    let mut steps = Steps::default();
    for (index, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let bad = |err: glave_event::DecodeError| format!("event line {}: {err}", index + 1);
        let event = decode(RawEvent::from_json(line).map_err(bad)?).map_err(bad)?;
        threads.insert(event.header.thread);
        trace.add(event.body, &mut steps);
    }
    trace.threads = threads.len();
    Ok(trace)
}

impl Trace {
    /// Adds one event.
    fn add(&mut self, body: Body, steps: &mut Steps) {
        match body {
            Body::TurnStarted(_) => {
                self.turns = self.turns.saturating_add(1);
                self.answer.clear();
            }
            Body::MessageAdded(message) if message.role == Role::Assistant => {
                self.answer = text_of(&message.parts);
            }
            Body::ToolCallRequested(call) => {
                self.tool_calls = self.tool_calls.saturating_add(1);
                let arguments = serde_json::to_string(&call.arguments).unwrap_or_default();
                steps.open.insert(call.call_id, (call.tool, arguments));
            }
            Body::ToolCallCompleted(done) => self.finished(&done, steps),
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
            Body::GoalEnded(ended) => self.goal = Some(ended.reason),
            Body::ThreadCompacted(_) => self.compactions = self.compactions.saturating_add(1),
            _ => {}
        }
    }

    /// Adds a finished tool call: its error, what a command printed, and whether it repeats one.
    fn finished(&mut self, done: &ToolCallCompleted, steps: &mut Steps) {
        if done.is_error {
            self.tool_errors = self.tool_errors.saturating_add(1);
        }
        let Some((tool, arguments)) = steps.open.remove(&done.call_id) else {
            return;
        };
        let result = text_of(&done.output);
        if tool == "bash" {
            self.printed.push_str(&result);
            self.printed.push('\n');
        }
        if !steps.done.insert((tool, arguments, result)) {
            self.repeated_steps = self.repeated_steps.saturating_add(1);
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
    const GOLDEN: &str = include_str!("../fixtures/golden.jsonl");

    #[test]
    fn the_golden_thread_reads_as_one_completed_turn() {
        let trace = parse(GOLDEN).unwrap();
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
                printed: "README.md\nsrc\n".to_owned(),
                goal: None,
                compactions: 0,
                repeated_steps: 0,
            }
        );
    }

    /// One `bash` call and its result, as JSON Lines.
    fn step(id: &str, command: &str, result: &str) -> String {
        let thread = "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10";
        let line = |number: u32, kind: &str, data: serde_json::Value| {
            serde_json::json!({"glave": 1, "id": format!("0192f5c2-7000-7000-8000-{number:012}"),
                "thread": thread, "seq": number, "at": "2026-10-07T21:14:00.001Z", "type": kind,
                "v": 1, "data": data})
            .to_string()
        };
        let requested = serde_json::json!({"call_id": id, "tool": "bash",
            "arguments": {"json": {"command": command}}});
        let completed = serde_json::json!({"call_id": id, "is_error": false,
            "output": [{"type": "text", "text": result}]});
        format!(
            "{}\n{}\n",
            line(1, "tool.call.requested", requested),
            line(2, "tool.call.completed", completed)
        )
    }

    #[test]
    fn a_step_repeats_only_when_it_is_the_same_call_with_the_same_result() {
        let steps = [
            step("a", "cat x", "one"),
            step("b", "cat x", "one"),
            step("c", "cat x", "two"),
            step("d", "cat y", "one"),
            step("e", "cat x", "one"),
        ];
        let trace = parse(&steps.concat()).unwrap();
        assert_eq!(trace.tool_calls, 5);
        assert_eq!(trace.repeated_steps, 2, "b and e; a new result is new");
        assert_eq!(trace.printed, "one\none\ntwo\none\none\n");
    }

    #[test]
    fn each_restart_from_a_summary_is_counted() {
        let compacted = |seq: u32| {
            serde_json::json!({"glave": 1, "id": format!("0192f5c2-7000-7000-8000-{seq:012}"),
                "thread": "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10", "seq": seq,
                "at": "2026-10-07T21:14:00.001Z", "type": "thread.compacted", "v": 1,
                "data": {"through": 1, "parts": [{"type": "text", "text": "So far."}]}})
            .to_string()
        };
        let events = format!("{}\n{}\n", compacted(3), compacted(4));
        assert_eq!(parse(&events).unwrap().compactions, 2);
    }

    #[test]
    fn a_bad_line_names_its_number() {
        let first = GOLDEN.lines().next().unwrap();
        let error = parse(&format!("{first}\nnot json\n")).unwrap_err();
        assert!(error.starts_with("event line 2:"), "{error}");
    }
}
