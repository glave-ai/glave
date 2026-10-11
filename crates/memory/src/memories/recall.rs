//! `recall` (`specs/memory.md`, "Recall"): memories and past conversations found by keyword.
//! Memories are matched here, from the view the ledger already gives; conversations through the
//! store's own full-text index (`EventLog::search`). Both are scoped exactly as memory is: the
//! person's and this space's in a shared space, this space's only in an isolated one, nothing
//! when memory is off, never another organization's. A hit from untrusted tool output says so.

use std::collections::BTreeSet;

use glave_event::{At, Body, MemoryId, MemoryScope, Role, Seq, ThreadId, decode};
use glave_harness::{MemoryContext, MemoryError, memory_ledger};
use glave_ports::search::{query_terms, searchable, terms};
use glave_ports::{EventLog, Scope};

use super::{Memories, storage};
use crate::book::Book;

/// Hits one recall returns when the model does not say.
pub const DEFAULT_HITS: usize = 5;
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
        let readable = self.inner.rules.readable(on.person, on.scope);
        let mut memories = Vec::new();
        for (kind, ledger) in &readable {
            let found = self
                .with_book(*ledger, |book| matching(book, *kind, &wanted))
                .await?;
            memories.extend(found);
        }
        memories.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.at.cmp(&a.1.at)));
        let mut hits: Vec<Recalled> = memories.into_iter().map(|(_, hit)| hit).collect();
        hits.truncate(limit);
        // This space's conversations, then the person's own.
        for (_, ledger) in readable.iter().rev() {
            if hits.len() >= limit {
                break;
            }
            let room = limit.saturating_sub(hits.len());
            let found = self
                .conversations(*ledger, on.thread, query, &wanted, room)
                .await?;
            hits.extend(found);
        }
        Ok(hits)
    }

    /// Up to `room` hits in `ledger`'s conversations, leaving out the thread `asking`.
    async fn conversations(
        &self,
        ledger: Scope,
        asking: ThreadId,
        query: &str,
        wanted: &BTreeSet<String>,
        room: usize,
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
            if found.thread == asking || Some(found.thread) == memory_ledger() {
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
            let (Some(text), at) = (searchable(&raw), raw.at) else {
                continue;
            };
            let (said_by, untrusted) = match decode(raw).map(|event| event.body) {
                Ok(Body::MessageAdded(message)) => match message.role {
                    Role::User => ("person", false),
                    Role::Assistant => ("assistant", false),
                },
                Ok(Body::ToolCallCompleted(done)) => ("tool output", done.untrusted),
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
