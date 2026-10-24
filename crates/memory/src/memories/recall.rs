//! `recall` (`specs/memory.md`, "Recall"): memories and past conversations found by keyword.
//! Memories are matched here, from the view the ledger already gives; conversations through the
//! store's own full-text index (`EventLog::search`). Both are scoped exactly as memory is: the
//! person's and this project's in a shared project, this project's only in an isolated one, nothing
//! when memory is off, never another organization's, and never a deleted chat. Hits are ranked
//! together by how many of the query's words they share. A hit from untrusted tool output says
//! so.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use glave_event::{At, Body, MemoryId, MemoryScope, Role, Seq, ThreadId, decode};
use glave_ports::memories::{MemoryContext, MemoryError};
use glave_ports::search::{Found, query_terms, searchable_body, terms};
use glave_ports::stream::Reserved;
use glave_ports::{EventLog, Scope, Standing};

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
    /// What `query` finds for a thread of `on`, at most `limit` (at most [`MOST_HITS`]): those
    /// sharing the most of its words first; among equals, memories (newest first), then this
    /// project's conversations, then the person's (best match first). The thread asking, and
    /// every deleted chat, is left out.
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
        // This project's conversations, then the person's own (one ledger in their Chats), each
        // searched whatever memories matched: memories sharing one word (often the project's
        // name) must not crowd out the message that holds them all.
        for ledger in ledgers(&readable) {
            let found = self
                .conversations(ledger, on.thread, (query, &wanted), limit)
                .await?;
            ranked.extend(found);
        }
        // Stable: among equals, the order above.
        ranked.sort_by_key(|(shared, _)| Reverse(*shared));
        Ok(ranked.into_iter().take(limit).map(|(_, hit)| hit).collect())
    }

    /// Whether `thread`, found in `ledger`, is not a deleted chat: a deleted chat is hidden
    /// everywhere, for good (`specs/storage.md`, "Archive and delete"), recall included. Each
    /// thread is looked up once in `seen`. A view that cannot be read counts as deleted, so a
    /// failure never shows one; a past conversation with no view (one glave did not start) is
    /// never deleted.
    async fn kept(
        &self,
        ledger: Scope,
        thread: ThreadId,
        seen: &mut BTreeMap<ThreadId, bool>,
    ) -> bool {
        if let Some(kept) = seen.get(&thread) {
            return *kept;
        }
        let kept = match self.inner.log.thread_view(&ledger, thread).await {
            Ok(view) => view.is_none_or(|view| view.standing != Standing::Deleted),
            Err(_) => false,
        };
        seen.insert(thread, kept);
        kept
    }

    /// Whether `thread` is a chat started for the person a thread of `on` works for, in a
    /// project `on` reads, and not deleted: the only chat a forget's result tells of.
    pub(crate) async fn own_chat(&self, on: &MemoryContext, thread: ThreadId) -> bool {
        let Some(person) = on.person else {
            return false;
        };
        let readable = self.rules().readable(on.person, on.scope);
        for ledger in ledgers(&readable) {
            if let Ok(Some(view)) = self.inner.log.thread_view(&ledger, thread).await {
                return view.standing != Standing::Deleted && view.whose() == Some(person);
            }
        }
        false
    }

    /// The hits among the index's best few for `limit` in `ledger`'s conversations, best match
    /// first, each with how many of `wanted` it shares, leaving out the thread `asking` and
    /// deleted chats.
    async fn conversations(
        &self,
        ledger: Scope,
        asking: ThreadId,
        (query, wanted): (&str, &BTreeSet<String>),
        limit: usize,
    ) -> Result<Vec<(usize, Recalled)>, MemoryError> {
        let log = &self.inner.log;
        // A few more than needed, since the asking thread's own events are skipped and a hit
        // further down may share more words.
        let found = log
            .search(&ledger, query, limit.saturating_mul(2).saturating_add(4))
            .await
            .map_err(storage)?;
        let (mut hits, mut seen) = (Vec::new(), BTreeMap::new());
        for found in found {
            if found.thread == asking
                || found.thread == Reserved::Memory.id()
                || !self.kept(ledger, found.thread, &mut seen).await
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
    async fn said_at(&self, ledger: Scope, found: Found) -> Option<Said> {
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
struct Said {
    text: String,
    /// `person`, `assistant` or `tool output`.
    by: &'static str,
    /// Output from outside that no one vouches for.
    untrusted: bool,
    at: At,
}

/// The ledgers whose conversations a thread reading `readable` searches: this project's, then
/// the person's own (one ledger in their Chats).
fn ledgers(readable: &[(MemoryScope, Scope)]) -> Vec<Scope> {
    let mut ledgers: Vec<Scope> = readable.iter().rev().map(|(_, ledger)| *ledger).collect();
    ledgers.dedup();
    ledgers
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
