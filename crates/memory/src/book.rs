//! The memories of one space, as its memory ledger says so far: each memory's versions, who
//! changed it, and whether it is forgotten. A view built from the events alone, never stored.
//! A forgotten memory's text leaves the view the moment its key is destroyed: its events read
//! as erased from then on, and a cached copy is dropped when the forget is read.

use std::collections::{BTreeMap, BTreeSet};

use glave_event::{
    At, Body, EventId, MemoryId, MemoryScope, MemorySource, Principal, PrincipalKind, RawEvent,
    decode,
};
use glave_harness::Remembered;

/// One version of a memory's text.
#[derive(Debug, Clone)]
pub(crate) struct Version {
    pub(crate) text: String,
    pub(crate) at: At,
    pub(crate) event: EventId,
    pub(crate) by: Principal,
    /// Undone by a `memory.restored`: no longer the memory's text.
    pub(crate) undone: bool,
}

/// A forget that has not been undone.
#[derive(Debug, Clone)]
pub(crate) struct Forgot {
    pub(crate) event: EventId,
    pub(crate) erase_at: At,
    pub(crate) by: Principal,
}

/// One memory and its history.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) memory: MemoryId,
    pub(crate) scope: MemoryScope,
    pub(crate) versions: Vec<Version>,
    pub(crate) source: Option<MemorySource>,
    pub(crate) untrusted_read: bool,
    pub(crate) forgot: Option<Forgot>,
}

impl Entry {
    /// The version in force, and its number (1 for the text saved).
    pub(crate) fn current(&self) -> Option<(u32, &Version)> {
        self.versions
            .iter()
            .enumerate()
            .rev()
            .find(|(_, version)| !version.undone)
            .and_then(|(index, version)| {
                Some((u32::try_from(index).ok()?.checked_add(1)?, version))
            })
    }

    /// The memory at version `number`, as it is shown.
    pub(crate) fn at(&self, number: u32) -> Option<Remembered> {
        let index = usize::try_from(number.checked_sub(1)?).ok()?;
        let version = self.versions.get(index)?;
        let saved = self.versions.first()?;
        Some(Remembered {
            memory: self.memory,
            version: number,
            scope: self.scope,
            text: version.text.clone(),
            saved_at: saved.at,
            edited_at: (index > 0).then_some(version.at),
            untrusted_read: self.untrusted_read,
            source: self.source,
        })
    }

    /// The memory as it is now, unless it is forgotten.
    pub(crate) fn live(&self) -> Option<Remembered> {
        if self.forgot.is_some() {
            return None;
        }
        let (number, _) = self.current()?;
        self.at(number)
    }

    /// When it was last saved or edited, for retention.
    pub(crate) fn touched(&self) -> Option<At> {
        self.current().map(|(_, version)| version.at)
    }
}

/// What one space's memory ledger says.
#[derive(Debug, Default)]
pub(crate) struct Book {
    /// The `seq` the next event will take.
    next: u64,
    /// Memories by id, in the order they were saved (ids are time-ordered).
    pub(crate) entries: BTreeMap<MemoryId, Entry>,
    /// Memories forgotten for good whose text was still readable when the forget was read:
    /// their keys are destroyed (again, harmlessly) by the next housekeeping, in case the
    /// writer that forgot them failed to.
    pub(crate) erasing: BTreeSet<MemoryId>,
}

impl Book {
    pub(crate) fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// The memories in force, oldest first.
    pub(crate) fn live(&self) -> Vec<Remembered> {
        self.entries.values().filter_map(Entry::live).collect()
    }

    /// Takes in the event at the next `seq`; any other is already in, or not yet.
    pub(crate) fn apply(&mut self, raw: RawEvent) {
        if raw.seq.get() != self.next_seq() {
            return;
        }
        self.next = self.next_seq().saturating_add(1);
        let (at, id) = (raw.at, raw.id);
        // A memory whose key was destroyed reads as erased, and its text never comes back.
        let Ok(event) = decode(raw) else {
            return;
        };
        match event.body {
            Body::MemorySaved(saved) => {
                let version = Version {
                    text: saved.text,
                    at,
                    event: id,
                    by: saved.by,
                    undone: false,
                };
                self.entries.insert(
                    saved.memory,
                    Entry {
                        memory: saved.memory,
                        scope: saved.scope,
                        versions: vec![version],
                        source: saved.source,
                        untrusted_read: saved.untrusted_read,
                        forgot: None,
                    },
                );
            }
            Body::MemoryEdited(edited) => {
                if let Some(entry) = self.entries.get_mut(&edited.memory) {
                    entry.versions.push(Version {
                        text: edited.text,
                        at,
                        event: id,
                        by: edited.by,
                        undone: false,
                    });
                }
            }
            Body::MemoryForgotten(forgotten) => {
                // Erased at once: the text leaves this view now, as it leaves the store.
                if forgotten.erase_at <= at {
                    if self.entries.remove(&forgotten.memory).is_some() {
                        self.erasing.insert(forgotten.memory);
                    }
                } else if let Some(entry) = self.entries.get_mut(&forgotten.memory) {
                    entry.forgot = Some(Forgot {
                        event: id,
                        erase_at: forgotten.erase_at,
                        by: forgotten.by,
                    });
                }
            }
            Body::MemoryRestored(restored) => {
                if let Some(entry) = self.entries.get_mut(&restored.memory) {
                    if entry
                        .forgot
                        .as_ref()
                        .is_some_and(|forgot| forgot.event == restored.undone)
                    {
                        entry.forgot = None;
                    }
                    for version in entry.versions.iter_mut().skip(1) {
                        if version.event == restored.undone {
                            version.undone = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Whether `by` is glave itself: its changes can be undone within 30 days.
pub(crate) fn by_glave(by: &Principal) -> bool {
    by.kind == PrincipalKind::Glave
}
