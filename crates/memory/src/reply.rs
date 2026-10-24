//! A memory tool's reply by reference (`specs/memory.md`, "Seeing, editing and forgetting"):
//! what a result showed, as ids, versions and places, and its words. The model reads the words
//! with the text of each memory and past conversation shown; a thread's log keeps them without
//! it (and without the query, the model's own words), and the reply in the result's
//! `structured`, versioned with its event, from which the text is read again whenever the
//! thread is. So neither the log nor its search index ever holds a copy of a memory's text that
//! a forget cannot reach, nor of a past conversation's that deleting it cannot.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glave_event::{At, MemoryId, MemoryScope, Seq, ThreadId};
use glave_ports::memories::{Remembered, Shown};
use serde::{Deserialize, Serialize};

use crate::memories::{Recalled, Source};

/// What a result showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub(crate) enum Reply {
    /// `remember` saved a memory, replacing `replaced` when it names one.
    Remembered {
        memory: Mentioned,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replaced: Option<MemoryId>,
    },
    /// `edit` changed a memory's text.
    Edited { memory: Mentioned },
    /// `remember` saved nothing: these memories are close to it.
    Similar { similar: Vec<Mentioned> },
    /// `recall` found `hits` for `query`.
    Recalled { query: String, hits: Vec<Hit> },
}

/// A memory a reply names, at the version it showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mentioned {
    pub(crate) memory: MemoryId,
    pub(crate) version: u32,
    pub(crate) scope: MemoryScope,
}

impl Mentioned {
    /// `memory`, named at its version.
    pub(crate) fn of(memory: &Remembered) -> Self {
        Self {
            memory: memory.memory,
            version: memory.version,
            scope: memory.scope,
        }
    }

    fn shown(self) -> Shown {
        Shown {
            memory: self.memory,
            version: self.version,
        }
    }

    /// `[id] (scope)`.
    fn line(self) -> String {
        format!("[{}] ({})", self.memory, self.scope.as_str())
    }
}

/// One thing `recall` found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub(crate) enum Hit {
    /// A memory in force, at the version found.
    Memory {
        memory: MemoryId,
        version: u32,
        scope: MemoryScope,
        at: At,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        untrusted_read: bool,
    },
    /// A message or a tool's output in a past thread, at `seq`.
    Thread {
        thread: ThreadId,
        seq: Seq,
        said_by: String,
        at: At,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        untrusted: bool,
    },
}

impl Hit {
    /// What recall found, named by its source.
    pub(crate) fn of(hit: &Recalled) -> Self {
        match hit.source {
            Source::Memory {
                memory,
                version,
                scope,
                untrusted_read,
            } => Self::Memory {
                memory,
                version,
                scope,
                at: hit.at,
                untrusted_read,
            },
            Source::Thread {
                thread,
                seq,
                said_by,
            } => Self::Thread {
                thread,
                seq,
                said_by: said_by.to_owned(),
                at: hit.at,
                untrusted: hit.untrusted,
            },
        }
    }

    /// What its text is of.
    pub(crate) fn shows(&self) -> Of {
        match *self {
            Self::Memory {
                memory, version, ..
            } => Of::Memory(Shown { memory, version }),
            Self::Thread { thread, seq, .. } => Of::Turn(thread, seq),
        }
    }

    /// Where it came from, as the model reads it: `[memory id] (scope, day)`, or `[thread id #seq]
    /// (who said it, day)`, with a caution when it came from outside.
    fn source(&self) -> String {
        match self {
            Self::Memory {
                memory,
                scope,
                at,
                untrusted_read,
                ..
            } => {
                let caution = if *untrusted_read {
                    ", saved after reading untrusted content"
                } else {
                    ""
                };
                format!(
                    "[memory {memory}] ({}, {}{caution})",
                    scope.as_str(),
                    day(*at)
                )
            }
            Self::Thread {
                thread,
                seq,
                said_by,
                at,
                untrusted,
            } => {
                let caution = if *untrusted { ", untrusted" } else { "" };
                format!(
                    "[thread {thread} #{}] ({said_by}{caution}, {})",
                    seq.get(),
                    day(*at)
                )
            }
        }
    }
}

/// What a text is of: a memory at one version, or the event of a past thread at `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Of {
    Memory(Shown),
    Turn(ThreadId, Seq),
}

/// The texts a reply shows, by what each is of.
pub(crate) type Texts = BTreeMap<Of, String>;

impl Reply {
    /// The memories it names, at the versions it showed.
    pub(crate) fn memories(&self) -> Vec<Shown> {
        match self {
            Self::Remembered { memory, .. } | Self::Edited { memory } => vec![memory.shown()],
            Self::Similar { similar } => similar.iter().map(|memory| memory.shown()).collect(),
            Self::Recalled { hits, .. } => hits
                .iter()
                .filter_map(|hit| match hit.shows() {
                    Of::Memory(shown) => Some(shown),
                    Of::Turn(..) => None,
                })
                .collect(),
        }
    }
}

/// The reply in words. With `texts`, as the model reads it: each text that can still be read,
/// and the query. With none, as the log keeps it: neither.
pub(crate) fn words(reply: &Reply, texts: Option<&Texts>) -> String {
    let text = |of: Of| {
        texts
            .and_then(|texts| texts.get(&of))
            .map_or_else(String::new, |text| format!(" {text}"))
    };
    match reply {
        Reply::Remembered { memory, replaced } => {
            let also = replaced.map_or_else(String::new, |old| format!(", replacing [{old}]"));
            let shown = text(Of::Memory(memory.shown()));
            format!("Remembered {}{shown}{also}.", memory.line())
        }
        Reply::Edited { memory } => {
            let shown = text(Of::Memory(memory.shown()));
            format!("Edited {}{shown}.", memory.line())
        }
        Reply::Similar { similar } => {
            let mut words = "Not saved: a similar memory exists:".to_owned();
            for memory in similar {
                let shown = text(Of::Memory(memory.shown()));
                let _ = write!(words, "\n- {}{shown}", memory.line());
            }
            words.push_str(
                "\nReplace it (`replaces`), merge them with edit, or save anyway with \
                 `force: true`.",
            );
            words
        }
        Reply::Recalled { query, hits } => {
            let asked = texts.map_or_else(String::new, |_| format!(" for {query:?}"));
            if hits.is_empty() {
                return format!("Nothing found{asked}.");
            }
            let mut words = format!("Found{asked}:");
            for hit in hits {
                let _ = write!(words, "\n- {}{}", hit.source(), text(hit.shows()));
            }
            words
        }
    }
}

fn day(at: At) -> String {
    at.to_string().chars().take(10).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 | n)
    }

    fn at() -> At {
        At::try_from("2026-10-04T10:00:00Z".to_owned()).unwrap()
    }

    #[test]
    fn the_log_keeps_the_words_without_any_text_or_the_query() {
        let memory = MemoryId::try_from(id(1)).unwrap();
        let thread = ThreadId::try_from(id(2)).unwrap();
        let seq = Seq::new(3).unwrap();
        let reply = Reply::Recalled {
            query: "flower marigold".into(),
            hits: vec![
                Hit::Memory {
                    memory,
                    version: 2,
                    scope: MemoryScope::Personal,
                    at: at(),
                    untrusted_read: false,
                },
                Hit::Thread {
                    thread,
                    seq,
                    said_by: "tool output".into(),
                    at: at(),
                    untrusted: true,
                },
            ],
        };
        let texts = Texts::from([
            (
                Of::Memory(Shown { memory, version: 2 }),
                "Likes marigold".into(),
            ),
            (Of::Turn(thread, seq), "marigold seeds".into()),
        ]);
        assert_eq!(
            words(&reply, Some(&texts)),
            format!(
                "Found for \"flower marigold\":\n- [memory {memory}] (personal, 2026-10-04) \
                 Likes marigold\n- [thread {thread} #3] (tool output, untrusted, 2026-10-04) \
                 marigold seeds"
            )
        );
        let kept = words(&reply, None);
        assert!(!kept.contains("marigold") && !kept.contains("flower"));
        assert!(kept.contains(&format!("[thread {thread} #3]")));
        let gone = Texts::from([(Of::Turn(thread, seq), "marigold seeds".into())]);
        assert!(
            words(&reply, Some(&gone)).contains("(personal, 2026-10-04)\n- [thread"),
            "a memory that cannot be read again is named without its text"
        );
        let stored = serde_json::to_value(&reply).unwrap();
        assert_eq!(serde_json::from_value::<Reply>(stored).unwrap(), reply);
    }

    #[test]
    fn a_save_an_edit_and_a_near_duplicate_name_their_memories_without_the_text() {
        let memory = Mentioned {
            memory: MemoryId::try_from(id(1)).unwrap(),
            version: 1,
            scope: MemoryScope::Project,
        };
        let old = MemoryId::try_from(id(4)).unwrap();
        let texts = Texts::from([(Of::Memory(memory.shown()), "Use tabs".to_owned())]);
        let line = format!("[{}] (project)", memory.memory);
        for (reply, read, kept) in [
            (
                Reply::Remembered {
                    memory,
                    replaced: Some(old),
                },
                format!("Remembered {line} Use tabs, replacing [{old}]."),
                format!("Remembered {line}, replacing [{old}]."),
            ),
            (
                Reply::Edited { memory },
                format!("Edited {line} Use tabs."),
                format!("Edited {line}."),
            ),
            (
                Reply::Similar {
                    similar: vec![memory],
                },
                format!(
                    "Not saved: a similar memory exists:\n- {line} Use tabs\nReplace it \
                     (`replaces`), merge them with edit, or save anyway with `force: true`."
                ),
                format!(
                    "Not saved: a similar memory exists:\n- {line}\nReplace it (`replaces`), \
                     merge them with edit, or save anyway with `force: true`."
                ),
            ),
        ] {
            assert_eq!(words(&reply, Some(&texts)), read);
            assert_eq!(words(&reply, None), kept);
            assert_eq!(reply.memories(), [memory.shown()]);
        }
    }
}
