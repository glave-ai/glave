//! What one thread's model is shown of memory, and when: the block at the start (in the first
//! message, after the project's instructions, before the person's words), a note at each later
//! turn whose memories changed meanwhile, and the results of memory calls that showed the text
//! of a memory or a past conversation. None of it is stored in the thread: it is laid over the
//! conversation each time it is sent, so the start is never rewritten and the log never holds
//! a memory's text, nor a copy a forget cannot reach (`specs/memory.md`, "Forgetting really
//! forgets").

use std::collections::BTreeMap;
use std::sync::Arc;

use glave_event::{CallId, MemoryId, Part, ToolCallCompleted};
use glave_ports::memories::{
    Kept, Memory, MemoryChange as Change, MemoryContext, Remembered, SHOWN, Shown,
};
use glave_ports::{BlobStore, EventLog, Message, ModelProvider};
use serde::{Deserialize, Serialize};

use super::render::{self, Line, changes};
use crate::approval::Approver;
use crate::harness::{Harness, Thread};
use crate::untrusted::labelled;

/// What a turn's configuration records about memory: the memories the model knew at its start,
/// at their versions. Never their text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConfigMemory {
    pub(crate) memories: Vec<Shown>,
    /// The contradictions tidy-up left for the person, among those memories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) questions: Vec<[MemoryId; 2]>,
}

/// A thread's memory, as its model is shown it.
#[derive(Debug, Clone, Default)]
pub(crate) struct MemoryState {
    /// The block at the start: in the first message, before its last part.
    pub(crate) start: Option<String>,
    /// Notes, each opening the user message at that index.
    pub(crate) notes: Vec<(usize, String)>,
    /// The memories the model knows now, with their text, by id.
    pub(crate) known: BTreeMap<MemoryId, Remembered>,
    /// The contradictions the model was told to ask the person about.
    pub(crate) questions: Vec<[MemoryId; 2]>,
    /// What the memory call being recorded changed and showed, for its `tool.call.completed`.
    pub(crate) structured: Option<serde_json::Value>,
    /// The words the memory call being recorded keeps in the log, in place of its result's first
    /// part, when that shows the text of a memory or a past conversation.
    pub(crate) kept: Option<String>,
    /// What the model reads in place of the first part of such a call's result, by call.
    pub(crate) results: BTreeMap<CallId, Part>,
}

impl MemoryState {
    /// The ids and versions the model knows.
    fn versions(&self) -> BTreeMap<MemoryId, u32> {
        self.known
            .iter()
            .map(|(id, memory)| (*id, memory.version))
            .collect()
    }

    /// Takes in what one of the thread's own memory calls did: it needs no note later.
    pub(crate) fn apply(&mut self, change: &Change) {
        match change {
            Change::Saved { memory, replaced } => {
                if let Some(replaced) = replaced {
                    self.known.remove(replaced);
                }
                self.known.insert(memory.memory, memory.clone());
            }
            Change::Edited(memory) => {
                self.known.insert(memory.memory, memory.clone());
            }
            Change::Forgotten(memory) => {
                self.known.remove(memory);
            }
        }
    }

    /// After a compaction: the start shows everything known, and the notes fold away, as do
    /// the results of calls no longer in the conversation.
    pub(crate) fn restart(&mut self) {
        let known: Vec<Remembered> = self.known.values().cloned().collect();
        self.questions = among(&self.questions, &known);
        self.start = render::start(&known, &self.questions);
        self.notes.clear();
        self.results.clear();
    }

    /// Takes in what the memory call being recorded did: what it changed and showed, for its
    /// `tool.call.completed`, and the words its result is kept in, when it showed any text.
    pub(crate) fn called(&mut self, change: Option<&Change>, kept: Option<Kept>) {
        self.structured = recorded(change, kept.as_ref());
        self.kept = kept.map(|kept| kept.text);
    }

    /// `completed` as the log keeps it: the result of the memory call being recorded with its
    /// first part in the words it is kept in, labelled as the output is, when it showed the
    /// text of a memory or a past conversation; and the part the model reads instead, for
    /// [`MemoryState::read_instead`] once the result is recorded.
    pub(crate) fn keep(
        &mut self,
        mut completed: ToolCallCompleted,
    ) -> (ToolCallCompleted, Option<Part>) {
        let Some(text) = self.kept.take() else {
            return (completed, None);
        };
        let kept = Part::Text { text };
        let kept = if completed.untrusted {
            labelled(kept)
        } else {
            kept
        };
        let read = (completed.output.first_mut()).map(|first| std::mem::replace(first, kept));
        (completed, read)
    }

    /// The model reads `read` in place of the first part of call `call_id`'s result, as the
    /// conversation is sent ([`overlay`]).
    pub(crate) fn read_instead(&mut self, call_id: CallId, read: Part) {
        self.results.insert(call_id, read);
    }
}

/// The pairs in `questions` whose memories are both in `memories`.
pub(crate) fn among(questions: &[[MemoryId; 2]], memories: &[Remembered]) -> Vec<[MemoryId; 2]> {
    questions
        .iter()
        .filter(|pair| {
            pair.iter()
                .all(|memory| memories.iter().any(|shown| shown.memory == *memory))
        })
        .copied()
        .collect()
}

/// What a memory call's `tool.call.completed` records in `structured`: what it changed, as ids
/// and versions only, and what its result showed, by reference (`shown`). `None` for neither.
pub(crate) fn recorded(change: Option<&Change>, kept: Option<&Kept>) -> Option<serde_json::Value> {
    let mut recorded = match change {
        Some(Change::Saved { memory, replaced }) => serde_json::json!({
            "memory": memory.memory,
            "version": memory.version,
            "replaced": replaced,
        }),
        Some(Change::Edited(memory)) => serde_json::json!({
            "memory": memory.memory,
            "version": memory.version,
        }),
        Some(Change::Forgotten(memory)) => serde_json::json!({
            "memory": memory,
            "forgotten": true,
        }),
        None => serde_json::json!({}),
    };
    let fields = recorded.as_object_mut()?;
    if let Some(kept) = kept {
        fields.insert(SHOWN.to_owned(), kept.shown.clone());
    }
    (!fields.is_empty()).then_some(recorded)
}

/// `messages` with memory laid over them, as the model is sent them.
pub(crate) fn overlay(messages: &[Message], state: &MemoryState) -> Vec<Message> {
    let mut messages = messages.to_vec();
    if let Some(start) = &state.start
        && let Some(Message::User { parts }) = messages.first_mut()
    {
        let at = parts.len().saturating_sub(1);
        parts.insert(
            at,
            Part::Text {
                text: start.clone(),
            },
        );
    }
    for (index, note) in &state.notes {
        if let Some(Message::User { parts }) = messages.get_mut(*index) {
            parts.insert(0, Part::Text { text: note.clone() });
        }
    }
    for message in &mut messages {
        if let Message::Tool {
            call_id, output, ..
        } = message
            && let Some(read) = state.results.get(call_id)
            && let Some(first) = output.first_mut()
        {
            first.clone_from(read);
        }
    }
    messages
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Memory for `thread`, and who and where it is for, when the thread reads it: never in a
    /// subagent's thread (the parent briefs it), never where the project's setting is off.
    pub(crate) fn memory_for(&self, thread: &Thread) -> Option<(Arc<dyn Memory>, MemoryContext)> {
        let memory = self.memory.as_ref()?;
        if thread.delegated() {
            return None;
        }
        let on = MemoryContext {
            person: thread.for_,
            agent: self.actors.agent,
            scope: thread.scope,
            thread: thread.id,
            seq: Some(thread.next_seq),
            untrusted_read: thread.untrusted || thread.read_secret,
        };
        memory.active(&on).then(|| (Arc::clone(memory), on))
    }

    /// Looks at the thread's memories as a turn starts: a new thread opens with them, and a
    /// thread whose memories changed since it last looked gets a note. Returns what the
    /// turn's configuration records. A store that fails leaves memory out of this turn.
    pub(crate) async fn look_at_memory(&self, thread: &mut Thread) -> Option<ConfigMemory> {
        let (memory, on) = self.memory_for(thread)?;
        let now = memory.list(&on).await.ok()?;
        let questions = if now.len() >= 2 {
            among(&memory.questions(&on).await.unwrap_or_default(), &now)
        } else {
            Vec::new()
        };
        if thread.messages.is_empty() {
            thread.memory.start = render::start(&now, &questions);
            thread.memory.notes.clear();
        } else {
            let mut lines = changes(&thread.memory.versions(), &now);
            lines.extend(
                questions
                    .iter()
                    .filter(|pair| !thread.memory.questions.contains(pair))
                    .map(|pair| Line::Asked(*pair)),
            );
            if let Some(note) = render::note(&lines) {
                thread.memory.notes.push((thread.messages.len(), note));
            }
        }
        thread.memory.known = now.iter().map(|m| (m.memory, m.clone())).collect();
        thread.memory.questions.clone_from(&questions);
        Some(ConfigMemory {
            memories: now.iter().map(Remembered::shown).collect(),
            questions,
        })
    }

    /// The conversation as the model is sent it: with memory, when the thread reads it.
    pub(crate) fn sent(&self, thread: &Thread) -> Vec<Message> {
        if self.memory_for(thread).is_some() {
            overlay(&thread.messages, &thread.memory)
        } else {
            thread.messages.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(texts: &[&str]) -> Message {
        Message::User {
            parts: texts
                .iter()
                .map(|text| Part::Text {
                    text: (*text).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_start_goes_after_the_instructions_and_notes_open_their_message() {
        let state = MemoryState {
            start: Some("MEMORY".into()),
            notes: vec![(2, "NOTE".into())],
            ..MemoryState::default()
        };
        let messages = vec![
            user(&["AGENTS", "first prompt"]),
            Message::Assistant {
                parts: Vec::new(),
                tool_calls: Vec::new(),
            },
            user(&["second prompt"]),
        ];
        let sent = overlay(&messages, &state);
        assert_eq!(
            sent.first(),
            Some(&user(&["AGENTS", "MEMORY", "first prompt"]))
        );
        assert_eq!(sent.get(2), Some(&user(&["NOTE", "second prompt"])));
        assert_eq!(overlay(&messages, &MemoryState::default()), messages);
    }

    #[test]
    fn a_kept_result_is_recorded_in_its_words_and_read_as_the_memory_gave_it() {
        let text = |text: &str| Part::Text { text: text.into() };
        let call_id = CallId::new("call_1").unwrap();
        let completed = |output: Vec<Part>| ToolCallCompleted {
            call_id: call_id.clone(),
            is_error: false,
            output,
            structured: None,
            untrusted: true,
        };
        let given = vec![
            text("Found for \"flower\": marigold"),
            text("[A hook says: ok]"),
        ];
        let mut state = MemoryState {
            kept: Some("Found:".into()),
            ..MemoryState::default()
        };
        let (recorded, read) = state.keep(completed(given.clone()));
        let Some(Part::Text { text: first }) = recorded.output.first() else {
            panic!("a text part");
        };
        assert!(first.contains("<untrusted>\nFound:\n") && !first.contains("marigold"));
        assert_eq!(recorded.output.get(1), given.get(1), "a hook's words stay");
        let messages = vec![Message::Tool {
            call_id: call_id.clone(),
            is_error: false,
            output: recorded.output,
        }];
        assert_eq!(
            overlay(&messages, &state),
            messages,
            "nothing is laid over a result not yet recorded"
        );
        state.read_instead(call_id.clone(), read.unwrap());
        let sent = overlay(&messages, &state);
        assert!(matches!(
            sent.first(),
            Some(Message::Tool { output, .. }) if *output == given
        ));
        let other = completed(given.clone());
        assert_eq!(
            state.keep(other.clone()),
            (other, None),
            "only the call kept is"
        );
        state.restart();
        assert_eq!(overlay(&messages, &state), messages);
    }
}
