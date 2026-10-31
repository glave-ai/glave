use serde_json::{Value, json};

use super::*;

/// A thread's events, made one after another, read as `thread/steps` reads them.
#[derive(Default)]
struct Thread {
    events: Vec<RawEvent>,
    calls: usize,
}

impl Thread {
    fn add(&mut self, kind: &str, data: &Value) -> &mut Self {
        let seq = self.events.len() + 1;
        let event = json!({
            "glave": 1,
            "id": format!("0192d6a0-0000-7000-8000-{seq:012}"),
            "thread": "0192d6a0-0000-7000-8000-000000000000",
            "seq": seq,
            "at": "2026-10-28T09:00:00Z",
            "type": kind,
            "v": 1,
            "data": data,
        });
        self.events
            .push(RawEvent::from_json(&event.to_string()).unwrap());
        self
    }

    fn starts(&mut self, prompt: &str) -> &mut Self {
        let config = json!({"sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08", "bytes": 2, "media_type": "application/json"});
        self.add(
            "turn.started",
            &json!({"model": {"provider": "p", "name": "m"}, "config": config}),
        );
        self.says("user", prompt)
    }

    fn ends(&mut self) -> &mut Self {
        self.add("turn.ended", &json!({"outcome": "completed"}))
    }

    fn says(&mut self, role: &str, text: &str) -> &mut Self {
        let parts = json!([{"type": "text", "text": text}]);
        self.add("message.added", &json!({"role": role, "parts": parts}))
    }

    /// The model thinks, and says nothing.
    fn thinks(&mut self) -> &mut Self {
        let parts = json!([{"type": "reasoning", "text": "hmm"}]);
        self.add(
            "message.added",
            &json!({"role": "assistant", "parts": parts}),
        )
    }

    /// A call of `tool`, of `kind`, naming `path`; `None` leaves it running, else whether it
    /// failed.
    fn calls(&mut self, tool: &str, kind: &str, path: &str, failed: Option<bool>) -> &mut Self {
        self.calls += 1;
        let id = format!("call_{}", self.calls);
        self.add(
            "tool.call.requested",
            &json!({"call_id": id, "tool": tool, "kind": kind, "arguments": {"json": {"path": path}}}),
        );
        if let Some(failed) = failed {
            let output = json!([{"type": "text", "text": "out"}]);
            self.add(
                "tool.call.completed",
                &json!({"call_id": id, "is_error": failed, "output": output}),
            );
        }
        self
    }

    fn steps(&self) -> ThreadSteps {
        let mut reading = Reading::default();
        for event in &self.events {
            reading.read(event.clone());
        }
        reading.done()
    }
}

fn lines(steps: &ThreadSteps) -> Vec<String> {
    steps
        .runs
        .iter()
        .map(|run| run.sentence.to_string())
        .collect()
}

#[test]
fn what_a_service_asks_stands_between_runs() {
    let mut thread = Thread::default();
    thread
        .starts("file it")
        .calls("mcp__x__y", "other", "", Some(false));
    thread.add(
        "service.asked",
        &json!({
            "call_id": "call_1", "service": "x", "ask": "x asks you, not the model",
            "message": "Which?", "fields": [],
            "state": {
                "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
                "bytes": 2, "media_type": "application/json"
            }
        }),
    );
    thread.calls("mcp__x__y", "other", "", Some(false)).ends();
    assert_eq!(thread.steps().runs.len(), 2);
}

#[test]
fn a_run_is_the_calls_between_words_told_by_kind_with_its_failures() {
    let mut thread = Thread::default();
    thread
        .starts("fix the test")
        .calls("bash", "command", "", Some(true))
        .thinks()
        .calls("bash", "command", "", Some(false))
        .says("assistant", "Found it.")
        .calls("read", "read", "a.rs", Some(false))
        .calls("read", "read", "a.rs", Some(false))
        .calls("edit", "edit", "a.rs", Some(false))
        .calls("search", "search", "", Some(false))
        .calls("bash", "command", "", Some(false))
        .calls("mcp__search__web_search_exa", "web", "", Some(false))
        .calls("mcp__x__y", "other", "", Some(false))
        .says("assistant", "Fixed.")
        .ends();
    let steps = thread.steps();
    assert_eq!(
        lines(&steps),
        [
            "Ran 2 commands (1 failed)",
            "Ran 1 command, read 1 file, ran 1 search, edited 1 file, used the web once, used 1 tool"
        ],
        "reasoning alone says nothing, and a file read twice is one file"
    );
    let first = steps.runs.first().unwrap();
    assert_eq!(first.first.get(), 3);
    let ids: Vec<&str> = first.calls.iter().map(CallId::as_str).collect();
    assert_eq!(ids, ["call_1", "call_2"]);
    assert_eq!(
        steps
            .replies
            .iter()
            .map(|seq| seq.get())
            .collect::<Vec<_>>(),
        [23],
        "the turn's last words are its reply; the words before are said along the way"
    );
}

#[test]
fn a_run_with_a_call_still_running_is_told_in_the_present_and_its_turn_has_no_reply_yet() {
    let mut thread = Thread::default();
    thread
        .starts("go")
        .says("assistant", "Looking.")
        .calls("read", "read", "a.rs", Some(false))
        .calls("bash", "command", "", None);
    let steps = thread.steps();
    assert_eq!(lines(&steps), ["Running 1 command, reading 1 file"]);
    assert_eq!(steps.replies, [], "a turn still running has no reply yet");
}

#[test]
fn turns_questions_and_the_person_s_words_part_runs_and_a_call_recorded_before_kinds_is_a_tool() {
    let mut thread = Thread::default();
    thread
        .starts("one")
        .calls("bash", "command", "", Some(false))
        .ends()
        .starts("two")
        .calls("bash", "command", "", Some(false))
        .calls("question", "other", "", Some(false))
        .calls("bash", "command", "", Some(false))
        .says("user", "also this")
        .calls("bash", "other", "", Some(false))
        .ends();
    let steps = thread.steps();
    assert_eq!(
        lines(&steps),
        [
            "Ran 1 command",
            "Ran 1 command",
            "Ran 1 command",
            "Used 1 tool"
        ]
    );
    assert_eq!(steps.replies, [], "a turn that said nothing has no reply");
}

#[test]
fn a_run_s_line_is_written_again_in_the_callers_language() {
    let mut thread = Thread::default();
    thread
        .starts("go")
        .calls("bash", "command", "", Some(true))
        .calls("read", "read", "a.rs", Some(false));
    let mut told = thread.steps().runs.remove(0).sentence;
    told.translate(crate::words::Language::Accented);
    assert!(
        told.starts_with("[Ŕåñ 1 çöɱɱåñð") && told.contains("[, ŕéåð 1 ƒîļé"),
        "each clause in the language: {}",
        &*told
    );
}
