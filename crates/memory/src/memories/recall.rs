//! `recall` (`specs/memory.md`, "Recall"): memories and past conversations found by keyword.
//! Memories are matched here, from the view the ledger already gives; conversations through the
//! store's own full-text index (`EventLog::search`). Both are scoped exactly as memory is: the
//! person's and this project's in a shared project, this project's only in an isolated one, nothing
//! when memory is off, never another organization's. A hit from untrusted tool output says so.

use std::collections::{BTreeMap, BTreeSet};

use glave_event::{At, Body, MemoryId, MemoryScope, MemorySource, Role, Seq, ThreadId, decode};
use glave_ports::memories::{MemoryContext, MemoryError, memory_ledger};
use glave_ports::search::{Found, query_terms, searchable_body, terms};
use glave_ports::{EventLog, Scope};

use super::{Memories, storage};
use crate::book::Book;

/// Hits one recall returns when the model does not say.
pub(crate) const DEFAULT_HITS: usize = 5;
/// Hits one recall returns at most.
pub const MOST_HITS: usize = 10;
/// Characters of a past conversation one hit shows, at most.
const SNIPPET_CHARS: usize = 400;
/// Characters shown before the first matching word.
const BEFORE_CHARS: usize = 120;

/// Where a hit came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A memory in force.
    Memory {
        memory: MemoryId,
        scope: MemoryScope,
        /// Saved in a thread that had read untrusted content.
        untrusted_read: bool,
    },
    /// A message or a tool's output in a past thread, at `seq`.
    Thread {
        thread: ThreadId,
        seq: Seq,
        /// Who said it: `person`, `assistant` or `tool output`.
        said_by: &'static str,
    },
}

/// One thing `recall` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recalled {
    pub source: Source,
    pub at: At,
    /// The memory, or the part of the conversation around the first word found.
    pub text: String,
    /// Content from outside that no one vouches for: reading it counts as reading untrusted
    /// content.
    pub untrusted: bool,
}

impl<L: EventLog + 'static> Memories<L> {
    /// What `query` finds for a thread of `on`: memories first, best match first, then past
    /// conversations, at most `limit` (at most [`MOST_HITS`]). The thread asking is left out.
    pub async fn recall(
        &self,
        on: &MemoryContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Recalled>, MemoryError> {
        let limit = limit.clamp(1, MOST_HITS);
        let wanted: BTreeSet<String> = query_terms(query).into_iter().collect();
        if wanted.is_empty() {
            return Err(MemoryError::Invalid(
                "recall needs a few distinctive keywords to search for".into(),
            ));
        }
        let readable = self.rules().readable(on.person, on.scope);
        let mut memories = Vec::new();
        // Where forgotten memories were saved: those turns are never returned.
        let mut hidden = Vec::new();
        for (kind, ledger) in &readable {
            let (found, sources) = self
                .with_book(*ledger, |book| {
                    let sources: Vec<MemorySource> = book.hidden.values().copied().collect();
                    (matching(book, *kind, &wanted), sources)
                })
                .await?;
            memories.extend(found);
            hidden.extend(sources);
        }
        memories.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.at.cmp(&a.1.at)));
        let mut hidden = Hidden::new(hidden);
        let mut hits: Vec<Recalled> = memories.into_iter().map(|(_, hit)| hit).collect();
        hits.truncate(limit);
        // This project's conversations, then the person's own.
        for (_, ledger) in readable.iter().rev() {
            if hits.len() >= limit {
                break;
            }
            let room = limit.saturating_sub(hits.len());
            let found = self
                .conversations(*ledger, on.thread, (query, &wanted), room, &mut hidden)
                .await?;
            hits.extend(found);
        }
        Ok(hits)
    }

    /// Up to `room` hits in `ledger`'s conversations, leaving out the thread `asking` and the
    /// turns `hidden` names.
    async fn conversations(
        &self,
        ledger: Scope,
        asking: ThreadId,
        (query, wanted): (&str, &BTreeSet<String>),
        room: usize,
        hidden: &mut Hidden,
    ) -> Result<Vec<Recalled>, MemoryError> {
        let log = &self.inner.log;
        // A few more than needed, since the asking thread's own events are skipped.
        let found = log
            .search(&ledger, query, room.saturating_mul(2).saturating_add(4))
            .await
            .map_err(storage)?;
        let mut hits = Vec::new();
        for found in found {
            if hits.len() >= room {
                break;
            }
            if found.thread == asking
                || Some(found.thread) == memory_ledger()
                || hidden.hides(&self.inner.log, ledger, found).await
            {
                continue;
            }
            let after = Seq::new(found.seq.get().saturating_sub(1)).ok();
            // An event whose key was destroyed (an erased month) no longer opens: skipped.
            let Ok(events) = log.read(&ledger, found.thread, after, 1).await else {
                continue;
            };
            let Some(raw) = events.into_iter().next().filter(|raw| raw.seq == found.seq) else {
                continue;
            };
            let at = raw.at;
            let Ok(event) = decode(raw) else {
                continue;
            };
            let Some(text) = searchable_body(&event.body) else {
                continue;
            };
            let (said_by, untrusted) = match &event.body {
                Body::MessageAdded(message) => match message.role {
                    Role::User => ("person", false),
                    Role::Assistant => ("assistant", false),
                },
                Body::ToolCallCompleted(done) => ("tool output", done.untrusted),
                _ => continue,
            };
            hits.push(Recalled {
                source: Source::Thread {
                    thread: found.thread,
                    seq: found.seq,
                    said_by,
                },
                at,
                text: snippet(&text, wanted),
                untrusted,
            });
        }
        Ok(hits)
    }
}

/// The turns where forgotten memories were saved: a forget takes the turn that gave the memory
/// out of `recall` too, though the conversation itself stays as it was.
struct Hidden {
    sources: Vec<MemorySource>,
    /// Each source's turn as `seq` bounds, once read.
    turns: BTreeMap<(ThreadId, Seq), (u64, u64)>,
}

impl Hidden {
    fn new(sources: Vec<MemorySource>) -> Self {
        Self {
            sources,
            turns: BTreeMap::new(),
        }
    }

    /// Whether `found` is in the turn a forgotten memory was saved in. A thread that cannot be
    /// read hides the whole of it, so a failure never shows what was forgotten.
    async fn hides<L: EventLog>(&mut self, log: &L, ledger: Scope, found: Found) -> bool {
        let sources: Vec<MemorySource> = self
            .sources
            .iter()
            .filter(|source| source.thread == found.thread)
            .copied()
            .collect();
        for source in sources {
            let key = (source.thread, source.seq);
            let (first, last) = if let Some(turn) = self.turns.get(&key) {
                *turn
            } else {
                let turn = turn_of(log, ledger, source).await.unwrap_or((1, u64::MAX));
                self.turns.insert(key, turn);
                turn
            };
            if (first..=last).contains(&found.seq.get()) {
                return true;
            }
        }
        false
    }
}

/// Events read at a time when finding a turn's bounds.
const TURN_PAGE: usize = 500;

/// The bounds of the turn holding `source`: from its `turn.started` (or the thread's start) to
/// its `turn.ended` (or the thread's end).
async fn turn_of<L: EventLog>(
    log: &L,
    ledger: Scope,
    source: MemorySource,
) -> Result<(u64, u64), MemoryError> {
    let at = source.seq.get();
    let (mut first, mut last) = (1, u64::MAX);
    let mut after = None;
    loop {
        let page = log
            .read(&ledger, source.thread, after, TURN_PAGE)
            .await
            .map_err(storage)?;
        let full = page.len() == TURN_PAGE;
        for event in &page {
            let seq = event.seq.get();
            match event.kind.as_str() {
                "turn.started" if seq <= at => first = seq,
                "turn.ended" if seq >= at => return Ok((first, seq)),
                _ => {}
            }
            last = seq;
        }
        after = page.last().map(|event| event.seq);
        if !full {
            return Ok((first, last.max(at)));
        }
    }
}

/// The memories of `kind` in `book` that share a word with `wanted`, with how many they share.
fn matching(book: &Book, kind: MemoryScope, wanted: &BTreeSet<String>) -> Vec<(usize, Recalled)> {
    book.live()
        .into_iter()
        .filter(|memory| memory.scope == kind)
        .filter_map(|memory| {
            let words = terms(&memory.text, usize::MAX);
            let shared = words.iter().filter(|word| wanted.contains(*word)).count();
            (shared > 0).then(|| {
                let hit = Recalled {
                    source: Source::Memory {
                        memory: memory.memory,
                        scope: memory.scope,
                        untrusted_read: memory.untrusted_read,
                    },
                    at: memory.edited_at.unwrap_or(memory.saved_at),
                    text: memory.text,
                    untrusted: false,
                };
                (shared, hit)
            })
        })
        .collect()
}

/// The part of `text` around the first word in `wanted`, on one line, at most
/// [`SNIPPET_CHARS`], with `…` where it was cut.
pub(crate) fn snippet(text: &str, wanted: &BTreeSet<String>) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut first = 0;
    let mut start = None;
    for (index, (at, c)) in flat.char_indices().enumerate() {
        if c.is_alphanumeric() {
            if start.is_none() {
                start = Some((index, at));
            }
            continue;
        }
        if let Some((char_index, byte)) = start.take()
            && is_wanted(flat.get(byte..at).unwrap_or_default(), wanted)
        {
            first = char_index;
            break;
        }
    }
    if let Some((char_index, byte)) = start
        && first == 0
        && is_wanted(flat.get(byte..).unwrap_or_default(), wanted)
    {
        first = char_index;
    }
    let from = first.saturating_sub(BEFORE_CHARS);
    let total = flat.chars().count();
    let shown: String = flat.chars().skip(from).take(SNIPPET_CHARS).collect();
    let head = if from > 0 { "…" } else { "" };
    let tail = if from.saturating_add(SNIPPET_CHARS) < total {
        "…"
    } else {
        ""
    };
    format!("{head}{shown}{tail}")
}

fn is_wanted(word: &str, wanted: &BTreeSet<String>) -> bool {
    terms(word, 1)
        .first()
        .is_some_and(|term| wanted.contains(term))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wanted(words: &[&str]) -> BTreeSet<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_snippet_shows_the_text_around_the_first_word_found() {
        assert_eq!(
            snippet("We deploy with Render.", &wanted(&["render"])),
            "We deploy with Render."
        );
        let long = format!(
            "{} The tests need make db first. {}",
            "x ".repeat(200),
            "y ".repeat(300)
        );
        let shown = snippet(&long, &wanted(&["db"]));
        assert!(shown.starts_with('…') && shown.ends_with('…'), "{shown}");
        assert!(shown.contains("The tests need make db first."), "{shown}");
        assert!(shown.chars().count() <= SNIPPET_CHARS + 2);
        assert_eq!(
            snippet("ends with deploys", &wanted(&["deploy"])),
            "ends with deploys"
        );
    }
}
