//! A memory tool's result in words, as the model reads it and its thread's log keeps it: each
//! memory by its id and scope, with its text, and each thing `recall` found by its source, with
//! what it says. Search never indexes these words (`glave_ports::memories::FROM_MEMORY`).

use std::fmt::Write as _;

use glave_event::{At, MemoryId};
use glave_ports::memories::Remembered;

use crate::memories::{Recalled, Source};

/// `remember` saved `memory`, replacing `replaced` when it names one.
pub(crate) fn remembered(memory: &Remembered, replaced: Option<MemoryId>) -> String {
    let also = replaced.map_or_else(String::new, |old| format!(", replacing [{old}]"));
    format!("Remembered {}{also}.", line(memory))
}

/// `edit` changed `memory`'s text.
pub(crate) fn edited(memory: &Remembered) -> String {
    format!("Edited {}.", line(memory))
}

/// `remember` saved nothing: `similar` are close to it.
pub(crate) fn similar(similar: &[Remembered]) -> String {
    let mut words = "Not saved: a similar memory exists:".to_owned();
    for memory in similar {
        let _ = write!(words, "\n- {}", line(memory));
    }
    words.push_str(
        "\nReplace it (`replaces`), merge them with edit, or save anyway with `force: true`.",
    );
    words
}

/// `recall` found `hits` for `query`.
pub(crate) fn recalled(query: &str, hits: &[Recalled]) -> String {
    if hits.is_empty() {
        return format!("Nothing found for {query:?}.");
    }
    let mut words = format!("Found for {query:?}:");
    for hit in hits {
        let _ = write!(words, "\n- {} {}", source(hit), hit.text);
    }
    words
}

/// `[id] (scope) text`.
fn line(memory: &Remembered) -> String {
    format!(
        "[{}] ({}) {}",
        memory.memory,
        memory.scope.as_str(),
        memory.text
    )
}

/// Where `hit` came from, as the model reads it: `[memory id] (scope, day)`, or `[thread id
/// #seq] (who said it, day)`, with a caution when it came from outside.
fn source(hit: &Recalled) -> String {
    match hit.source {
        Source::Memory {
            memory,
            scope,
            untrusted_read,
        } => {
            let caution = if untrusted_read {
                ", saved after reading untrusted content"
            } else {
                ""
            };
            format!(
                "[memory {memory}] ({}, {}{caution})",
                scope.as_str(),
                day(hit.at)
            )
        }
        Source::Thread {
            thread,
            seq,
            said_by,
        } => {
            let caution = if hit.untrusted { ", untrusted" } else { "" };
            format!(
                "[thread {thread} #{}] ({said_by}{caution}, {})",
                seq.get(),
                day(hit.at)
            )
        }
    }
}

fn day(at: At) -> String {
    at.to_string().chars().take(10).collect()
}

#[cfg(test)]
mod tests {
    use glave_event::{MemoryScope, Seq, ThreadId};

    use super::*;

    fn id(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 | n)
    }

    fn at() -> At {
        At::try_from("2026-10-04T10:00:00Z".to_owned()).unwrap()
    }

    #[test]
    fn a_recall_names_each_hit_by_its_source_with_what_it_says() {
        let memory = MemoryId::try_from(id(1)).unwrap();
        let thread = ThreadId::try_from(id(2)).unwrap();
        let hits = [
            Recalled {
                source: Source::Memory {
                    memory,
                    scope: MemoryScope::Personal,
                    untrusted_read: false,
                },
                at: at(),
                text: "Likes marigold".into(),
                untrusted: false,
            },
            Recalled {
                source: Source::Thread {
                    thread,
                    seq: Seq::new(3).unwrap(),
                    said_by: "tool output",
                },
                at: at(),
                text: "marigold seeds".into(),
                untrusted: true,
            },
        ];
        assert_eq!(
            recalled("flower marigold", &hits),
            format!(
                "Found for \"flower marigold\":\n- [memory {memory}] (personal, 2026-10-04) \
                 Likes marigold\n- [thread {thread} #3] (tool output, untrusted, 2026-10-04) \
                 marigold seeds"
            )
        );
        assert_eq!(recalled("oak", &[]), "Nothing found for \"oak\".");
    }

    #[test]
    fn a_save_an_edit_and_a_near_duplicate_name_their_memories_with_the_text() {
        let memory = Remembered {
            memory: MemoryId::try_from(id(1)).unwrap(),
            version: 1,
            scope: MemoryScope::Project,
            text: "Use tabs".into(),
            saved_at: at(),
            edited_at: None,
            untrusted_read: false,
            source: None,
        };
        let old = MemoryId::try_from(id(4)).unwrap();
        let line = format!("[{}] (project) Use tabs", memory.memory);
        assert_eq!(
            remembered(&memory, Some(old)),
            format!("Remembered {line}, replacing [{old}].")
        );
        assert_eq!(edited(&memory), format!("Edited {line}."));
        assert_eq!(
            similar(&[memory]),
            format!(
                "Not saved: a similar memory exists:\n- {line}\nReplace it (`replaces`), merge \
                 them with edit, or save anyway with `force: true`."
            )
        );
    }
}
