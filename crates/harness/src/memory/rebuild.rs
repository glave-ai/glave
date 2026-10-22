//! Rebuilding what a resumed thread's model was shown of memory, from its events alone: each
//! turn's configuration names the memories it knew, at their versions; the thread's own memory
//! calls record what they changed, and what their results showed; a compaction restarts the
//! start. The text comes from the memories at those versions, and the past conversations as
//! they read now, so the start and every result read exactly as before, except that a forgotten
//! memory, which can no longer be opened, drops out, as does a turn `recall` hides now.

use std::collections::{BTreeMap, HashMap, HashSet};

use glave_event::{Body, CallId, Event, MemoryId, Part, Sha256};
use glave_ports::memories::{
    EDIT_MEMORY, FORGET, Memory, MemoryContext, RECALL, REMEMBER, Remembered, SHOWN, Shown,
};
use glave_ports::{BlobStore, EventLog, Message, ModelProvider, read_all};
use serde::Deserialize;

use super::render::{self, Line};
use super::state::{ConfigMemory, MemoryState, among};
use crate::approval::Approver;
use crate::conversation::follow;
use crate::harness::{Harness, Thread};
use crate::turn::CONFIG_LIMIT;
use crate::untrusted::labelled;

/// What a turn's configuration says about memory.
#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    memory: Option<ConfigMemory>,
}

/// One note, before its text is read: where it goes and what it says.
type Noted = (usize, Vec<Change>);

#[derive(Debug, Clone, Copy)]
enum Change {
    Added(Shown),
    Changed(Shown),
    Forgotten(MemoryId),
    Asked([MemoryId; 2]),
}

/// What a thread was shown, as ids and versions.
#[derive(Default)]
struct Trace {
    start: Vec<Shown>,
    /// The contradictions the start asked about.
    asked: Vec<[MemoryId; 2]>,
    notes: Vec<Noted>,
    known: BTreeMap<MemoryId, u32>,
    /// The contradictions known now.
    questions: Vec<[MemoryId; 2]>,
    /// The memory calls in the conversation whose results were kept by reference.
    results: Vec<KeptResult>,
}

/// A memory call's result the log keeps by reference.
struct KeptResult {
    call_id: CallId,
    /// What it showed, as its `structured` records it.
    shown: serde_json::Value,
    /// Its output is labelled untrusted.
    untrusted: bool,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Rebuilds `thread`'s memory from `events`, when the thread reads memory. A store that
    /// fails leaves the thread without it, as a turn that cannot list memories does.
    pub(crate) async fn rebuild_memory(&self, thread: &mut Thread, events: &[Event]) {
        let Some((memory, on)) = self.memory_for(thread) else {
            return;
        };
        let trace = self.trace(thread, events).await;
        let mut wanted: Vec<Shown> = trace.start.clone();
        for (_, changes) in &trace.notes {
            wanted.extend(changes.iter().filter_map(|change| match change {
                Change::Added(shown) | Change::Changed(shown) => Some(*shown),
                Change::Forgotten(_) | Change::Asked(_) => None,
            }));
        }
        wanted.extend(trace.known.iter().map(|(memory, version)| Shown {
            memory: *memory,
            version: *version,
        }));
        wanted.sort();
        wanted.dedup();
        let Ok(found) = memory.versions(&on, &wanted).await else {
            return;
        };
        let texts: HashMap<Shown, Remembered> = found.into_iter().map(|m| (m.shown(), m)).collect();
        let pick = |shown: &Shown| texts.get(shown).cloned();
        let start: Vec<Remembered> = trace.start.iter().filter_map(pick).collect();
        let notes = trace.notes.iter().filter_map(|(index, changes)| {
            let lines: Vec<Line> = changes
                .iter()
                .filter_map(|change| match change {
                    Change::Added(shown) => pick(shown).map(Line::Added),
                    Change::Changed(shown) => pick(shown).map(Line::Changed),
                    Change::Forgotten(memory) => Some(Line::Forgotten(*memory)),
                    Change::Asked(pair) => Some(Line::Asked(*pair)),
                })
                .collect();
            render::note(&lines).map(|note| (*index, note))
        });
        let known: Vec<Remembered> = trace
            .known
            .iter()
            .filter_map(|(memory, version)| {
                pick(&Shown {
                    memory: *memory,
                    version: *version,
                })
            })
            .collect();
        let results = Self::reread(memory.as_ref(), &on, trace.results).await;
        thread.memory = MemoryState {
            start: render::start(&start, &among(&trace.asked, &start)),
            notes: notes.collect(),
            questions: among(&trace.questions, &known),
            known: known.into_iter().map(|m| (m.memory, m)).collect(),
            structured: None,
            kept: None,
            results,
        };
    }

    /// Walks the events as the conversation is rebuilt, noting what memory each point showed.
    async fn trace(&self, thread: &Thread, events: &[Event]) -> Trace {
        let mut trace = Trace::default();
        let mut messages: Vec<Message> = Vec::new();
        let mut configs: HashMap<Sha256, Option<ConfigMemory>> = HashMap::new();
        let mut memory_calls: HashSet<CallId> = HashSet::new();
        for event in events {
            match &event.body {
                Body::TurnStarted(started) => {
                    let sha = started.config.sha256.clone();
                    if !configs.contains_key(&sha) {
                        let read = self.config_memory(thread, &sha).await;
                        configs.insert(sha.clone(), read);
                    }
                    if let Some(Some(config)) = configs.get(&sha) {
                        trace.look(messages.len(), config);
                    }
                }
                Body::ToolCallRequested(requested) => {
                    let tool = requested.tool.trim().to_ascii_lowercase();
                    if [REMEMBER, EDIT_MEMORY, FORGET, RECALL].contains(&tool.as_str()) {
                        memory_calls.insert(requested.call_id.clone());
                    }
                }
                Body::ToolCallCompleted(done)
                    if !done.is_error && memory_calls.contains(&done.call_id) =>
                {
                    if let Some(structured) = &done.structured {
                        trace.own(structured);
                        if let Some(shown) = structured.get(SHOWN) {
                            trace.results.push(KeptResult {
                                call_id: done.call_id.clone(),
                                shown: shown.clone(),
                                untrusted: done.untrusted,
                            });
                        }
                    }
                }
                Body::ThreadCompacted(_) => {
                    trace.results.clear();
                    trace.start = trace
                        .known
                        .iter()
                        .map(|(memory, version)| Shown {
                            memory: *memory,
                            version: *version,
                        })
                        .collect();
                    trace.asked.clone_from(&trace.questions);
                    trace.notes.clear();
                }
                _ => {}
            }
            follow(&mut messages, event);
        }
        trace
    }

    /// What the model reads in place of each of `kept`'s first parts: its words read again, as
    /// `MemoryState::results` holds them. One that cannot be read again reads as the log keeps
    /// it.
    async fn reread(
        memory: &dyn Memory,
        on: &MemoryContext,
        kept: Vec<KeptResult>,
    ) -> BTreeMap<CallId, Part> {
        if kept.is_empty() {
            return BTreeMap::new();
        }
        let shown: Vec<serde_json::Value> = kept.iter().map(|kept| kept.shown.clone()).collect();
        let read = memory.reread(on, &shown).await.unwrap_or_default();
        kept.into_iter()
            .zip(read)
            .filter_map(|(kept, text)| {
                let read = Part::Text { text: text? };
                let read = if kept.untrusted { labelled(read) } else { read };
                Some((kept.call_id, read))
            })
            .collect()
    }

    /// What the configuration stored under `sha` says about memory; `None` when it says
    /// nothing or cannot be read.
    async fn config_memory(&self, thread: &Thread, sha: &Sha256) -> Option<ConfigMemory> {
        let mut blob = self.blobs.get(&thread.scope, sha).await.ok()??;
        let bytes = read_all(blob.bytes.as_mut(), CONFIG_LIMIT).await.ok()?;
        serde_json::from_slice::<Config>(&bytes).ok()?.memory
    }
}

impl Trace {
    /// A turn started knowing `now`; its user message will be at `index`.
    fn look(&mut self, index: usize, config: &ConfigMemory) {
        let now = config.memories.as_slice();
        if index == 0 {
            self.start = now.to_vec();
            self.asked.clone_from(&config.questions);
            self.notes.clear();
        } else {
            let mut changes = Vec::new();
            for shown in now {
                match self.known.get(&shown.memory) {
                    None => changes.push(Change::Added(*shown)),
                    Some(version) if *version != shown.version => {
                        changes.push(Change::Changed(*shown));
                    }
                    Some(_) => {}
                }
            }
            for memory in self.known.keys() {
                if !now.iter().any(|shown| shown.memory == *memory) {
                    changes.push(Change::Forgotten(*memory));
                }
            }
            changes.extend(
                config
                    .questions
                    .iter()
                    .filter(|pair| !self.questions.contains(pair))
                    .map(|pair| Change::Asked(*pair)),
            );
            if !changes.is_empty() {
                self.notes.push((index, changes));
            }
        }
        self.known = now.iter().map(|s| (s.memory, s.version)).collect();
        self.questions.clone_from(&config.questions);
    }

    /// One of the thread's own memory calls, as its result recorded it.
    fn own(&mut self, structured: &serde_json::Value) {
        #[derive(Deserialize)]
        struct Recorded {
            memory: MemoryId,
            #[serde(default)]
            version: Option<u32>,
            #[serde(default)]
            replaced: Option<MemoryId>,
            #[serde(default)]
            forgotten: bool,
        }
        let Ok(recorded) = serde_json::from_value::<Recorded>(structured.clone()) else {
            return;
        };
        if let Some(replaced) = recorded.replaced {
            self.known.remove(&replaced);
        }
        match (recorded.forgotten, recorded.version) {
            (true, _) => {
                self.known.remove(&recorded.memory);
            }
            (false, Some(version)) => {
                self.known.insert(recorded.memory, version);
            }
            (false, None) => {}
        }
    }
}
