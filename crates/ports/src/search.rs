//! Keyword search over conversations (`specs/memory.md`, "Recall"): the words an event is
//! found by, and what a search finds. Each store keeps every searchable event's words sealed, in
//! a row written in the same transaction as the event, and searches them in memory
//! ([`SearchCache`]); every adapter normalises words the same way, here. A thread's rows leave
//! the index in the transaction that deletes it ([`deletes_thread`]).

mod cache;

use std::collections::BTreeSet;

use glave_event::{
    Body, Part, Payload, RawEvent, Seq, ThreadDeleted, ThreadId, ToolCallCompleted, decode,
};

use crate::memories::FROM_MEMORY;

pub use self::cache::{
    Fingerprint, IndexRow, IndexRows, MOST_CACHED, MOST_LOADED, SQUARES_MODULUS, SearchCache,
};

/// Distinct words one event is indexed under, at most: a long output is found by its first.
const MOST_TERMS: usize = 2_048;
/// Words of one query searched for, at most.
const MOST_QUERY_TERMS: usize = 16;
/// Characters of the text one event is indexed by, at most.
const MOST_CHARS: usize = 200_000;
/// The longest word indexed, in characters: longer runs are ids and hashes, not words.
const LONGEST: usize = 40;

/// Words too common to find anything by.
const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "do", "for", "from", "has", "have", "in",
    "is", "it", "its", "of", "on", "or", "so", "that", "the", "this", "to", "was", "we", "with",
    "what", "which", "who", "how", "when", "where", "did", "does", "you", "your", "our", "my",
    "me", "i",
];

/// One event a search found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Found {
    pub thread: ThreadId,
    pub seq: Seq,
}

/// The distinct words of `text`, in the order they first appear, at most `most`: lowercase
/// letters and digits, a plural's `s` dropped, single letters and common words left out.
pub fn terms(text: &str, most: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut terms = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        if terms.len() >= most {
            break;
        }
        let length = word.chars().count();
        if !(2..=LONGEST).contains(&length) {
            continue;
        }
        let word = word.to_lowercase();
        if STOP.contains(&word.as_str()) {
            continue;
        }
        let word = match word.strip_suffix('s') {
            Some(stem) if stem.chars().count() > 3 && !stem.ends_with('s') => stem.to_owned(),
            _ => word,
        };
        if seen.insert(word.clone()) {
            terms.push(word);
        }
    }
    terms
}

/// The text `event` is found by, when it is one a search looks in ([`searchable_body`]).
fn searchable(event: &RawEvent) -> Option<String> {
    let kind = event.kind.as_str();
    if kind != "message.added" && kind != "tool.call.completed" {
        return None;
    }
    searchable_body(&decode(event.clone()).ok()?.body)
}

/// The text an event's `body` is found by, when it is one a search looks in: a message's text,
/// or a tool's output. Everything else (configuration, approvals, memory ledgers, and memory
/// tools' results, so a forgotten memory is never found through one) is never indexed.
pub fn searchable_body(body: &Body) -> Option<String> {
    let parts = match body {
        Body::MessageAdded(message) => &message.parts,
        Body::ToolCallCompleted(done) if !done.is_error && !from_memory(done) => &done.output,
        _ => return None,
    };
    let mut text = String::new();
    for part in parts {
        if let Part::Text { text: part } = part {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(part);
        }
        if text.len() >= MOST_CHARS {
            break;
        }
    }
    let cut: String = text.chars().take(MOST_CHARS).collect();
    (!cut.trim().is_empty()).then_some(cut)
}

/// Whether `done` is a memory tool's result ([`FROM_MEMORY`]).
fn from_memory(done: &ToolCallCompleted) -> bool {
    let marked = done.structured.as_ref().and_then(|s| s.get(FROM_MEMORY));
    marked.and_then(serde_json::Value::as_bool) == Some(true)
}

/// The words `event` is indexed under; empty for an event a search never looks in.
pub fn indexed(event: &RawEvent) -> Vec<String> {
    searchable(event).map_or_else(Vec::new, |text| terms(&text, MOST_TERMS))
}

/// Whether `events`, appended to one thread, delete it (`thread.deleted`): then every adapter
/// removes the thread's words from its index in the same transaction, so nothing of a deleted
/// chat stays searchable, or even countable, there (`specs/storage.md`, "Search index").
pub fn deletes_thread(events: &[RawEvent]) -> bool {
    events
        .iter()
        .any(|event| event.kind.as_str() == ThreadDeleted::TYPE)
}

/// How much a word found in `with` of `events` counts toward a match: BM25's inverse document
/// frequency, so a rare word outweighs a common one. Every store ranks by it.
pub(crate) fn weight(events: usize, with: usize) -> f64 {
    let count = |n: usize| f64::from(u32::try_from(n).unwrap_or(u32::MAX));
    let (events, with) = (count(events), count(with));
    ((events - with + 0.5) / (with + 0.5)).ln_1p()
}

/// The words a query searches for.
pub fn query_terms(query: &str) -> Vec<String> {
    terms(query, MOST_QUERY_TERMS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_lowercased_stemmed_and_counted_once() {
        assert_eq!(
            terms("The Tests need `make db` first; the TESTS pass.", 10),
            ["test", "need", "make", "db", "first", "pass"]
        );
        assert_eq!(terms("Deploys go to Render", 2), ["deploy", "go"]);
        assert_eq!(
            terms("x a 0123456789012345678901234567890123456789x", 5),
            Vec::<String>::new()
        );
        assert_eq!(query_terms("Where do we deploy?"), ["deploy"]);
    }

    proptest::proptest! {
        #[test]
        fn terms_are_bounded_and_distinct(text in ".{0,400}", most in 0usize..20) {
            let found = terms(&text, most);
            proptest::prop_assert!(found.len() <= most);
            let distinct: BTreeSet<&String> = found.iter().collect();
            proptest::prop_assert_eq!(distinct.len(), found.len());
            proptest::prop_assert!(found.iter().all(|t| t.chars().count() <= LONGEST));
        }
    }
}
