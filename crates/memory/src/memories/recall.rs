//! `recall` (`specs/memory.md`, "Recall"): memories and past conversations found by keyword.
//! Memories are matched here, from the view the ledger already gives; conversations through the
//! store's own full-text index (`EventLog::search`). Both are scoped exactly as memory is: the
//! person's and this project's in a shared project, this project's only in an isolated one, nothing
//! when memory is off, never another organization's. Hits are ranked together by how many of the
//! query's words they share. A hit from untrusted tool output says so. A thread keeps what
//! recall found by reference (`reply.rs`), and `reread.rs` reads its texts again.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use glave_event::{At, Body, MemoryId, MemoryScope, MemorySource, Role, Seq, ThreadId, decode};
use glave_ports::memories::{MemoryContext, MemoryError};
use glave_ports::search::{Found, query_terms, searchable_body, terms};
use glave_ports::stream::Reserved;
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
    /// A memory in force, at the version found.
    Memory {
        memory: MemoryId,
        version: u32,
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
    /// What `query` finds for a thread of `on`, at most `limit` (at most [`MOST_HITS`]): those
    /// sharing the most of its words first; among equals, memories (newest first), then this
    /// project's conversations, then the person's (best match first). The thread asking is left
    /// out.
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
        let mut ranked = Vec::new();
        for (kind, ledger) in &readable {
            let found = self
                .with_book(*ledger, |book| matching(book, *kind, &wanted))
                .await?;
            ranked.extend(found);
        }
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.at.cmp(&a.1.at)));
        // Where memories the person forgot were saved: those turns are never returned.
        let mut hidden = self.hidden(&readable).await?;
        // This project's conversations, then the person's own (one ledger in their Chats), each
        // searched whatever memories matched: memories sharing one word (often the project's
        // name) must not crowd out the message that holds them all.
        for ledger in ledgers(&readable) {
            let found = self
                .conversations(ledger, on.thread, (query, &wanted), limit, &mut hidden)
                .await?;
            ranked.extend(found);
        }
        // Stable: among equals, the order above.
        ranked.sort_by_key(|(shared, _)| Reverse(*shared));
        Ok(ranked.into_iter().take(limit).map(|(_, hit)| hit).collect())
    }

    /// The turns where memories the person forgot were saved, as the ledgers `readable` names
    /// record them.
    pub(super) async fn hidden(
        &self,
        readable: &[(MemoryScope, Scope)],
    ) -> Result<Hidden, MemoryError> {
        let mut sources = Vec::new();
        for (_, ledger) in readable {
            let here = self
                .with_book(*ledger, |book| {
                    book.hidden.values().copied().collect::<Vec<MemorySource>>()
                })
                .await?;
            sources.extend(here);
        }
        Ok(Hidden::new(sources))
    }

    /// The hits among the index's best few for `limit` in `ledger`'s conversations, best match
    /// first, each with how many of `wanted` it shares, leaving out the thread `asking` and the
    /// turns `hidden` names.
    async fn conversations(
        &self,
        ledger: Scope,
        asking: ThreadId,
        (query, wanted): (&str, &BTreeSet<String>),
        limit: usize,
        hidden: &mut Hidden,
    ) -> Result<Vec<(usize, Recalled)>, MemoryError> {
        let log = &self.inner.log;
        // A few more than needed, since the asking thread's own events are skipped and a hit
        // further down may share more words.
        let found = log
            .search(&ledger, query, limit.saturating_mul(2).saturating_add(4))
            .await
            .map_err(storage)?;
        let mut hits = Vec::new();
        for found in found {
            if found.thread == asking
                || found.thread == Reserved::Memory.id()
                || hidden.hides(log, ledger, found).await
            {
                continue;
            }
            let Some(said) = self.said_at(ledger, found).await else {
                continue;
            };
            let hit = Recalled {
                source: Source::Thread {
                    thread: found.thread,
                    seq: found.seq,
                    said_by: said.by,
                },
                at: said.at,
                text: snippet(&said.text, wanted),
                untrusted: said.untrusted,
            };
            hits.push((shared(&said.text, wanted), hit));
        }
        Ok(hits)
    }

    /// The event at `found` in `ledger`, as recall shows it; `None` when there is none there
    /// with text, or it no longer opens (an event whose key was destroyed, an erased month).
    pub(super) async fn said_at(&self, ledger: Scope, found: Found) -> Option<Said> {
        let after = Seq::new(found.seq.get().saturating_sub(1)).ok();
        let events = (self.inner.log.read(&ledger, found.thread, after, 1).await).ok()?;
        let raw = events
            .into_iter()
            .next()
            .filter(|raw| raw.seq == found.seq)?;
        let at = raw.at;
        let event = decode(raw).ok()?;
        let text = searchable_body(&event.body)?;
        let (by, untrusted) = match &event.body {
            Body::MessageAdded(message) => match message.role {
                Role::User => ("person", false),
                Role::Assistant => ("assistant", false),
            },
            Body::ToolCallCompleted(done) => ("tool output", done.untrusted),
            _ => return None,
        };
        Some(Said {
            text,
            by,
            untrusted,
            at,
        })
    }
}

/// One event of a past conversation, as recall shows it.
pub(super) struct Said {
    pub(super) text: String,
    /// `person`, `assistant` or `tool output`.
    by: &'static str,
    /// Output from outside that no one vouches for.
    untrusted: bool,
    at: At,
}

/// The ledgers whose conversations a thread reading `readable` searches: this project's, then
/// the person's own (one ledger in their Chats).
pub(super) fn ledgers(readable: &[(MemoryScope, Scope)]) -> Vec<Scope> {
    let mut ledgers: Vec<Scope> = readable.iter().rev().map(|(_, ledger)| *ledger).collect();
    ledgers.dedup();
    ledgers
}

/// The turns where memories the person forgot were saved (`Book::hidden`): their forget takes
/// the turn that gave the memory out of `recall` too, though the conversation stays as it was.
pub(super) struct Hidden {
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

    /// Whether `found` is in a turn the person's forgets hide. A thread that cannot be
    /// read hides the whole of it, so a failure never shows what was forgotten.
    pub(super) async fn hides<L: EventLog>(
        &mut self,
        log: &L,
        ledger: Scope,
        found: Found,
    ) -> bool {
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
            let shared = shared(&memory.text, wanted);
            (shared > 0).then(|| {
                let hit = Recalled {
                    source: Source::Memory {
                        memory: memory.memory,
                        version: memory.version,
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

/// How many of `wanted`'s words `text` holds.
fn shared(text: &str, wanted: &BTreeSet<String>) -> usize {
    terms(text, usize::MAX)
        .iter()
        .filter(|word| wanted.contains(*word))
        .count()
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
