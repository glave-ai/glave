//! The memories of one project, as its memory ledger says so far: each memory's versions, who
//! changed it, and whether it is forgotten. A view built from the events alone, never stored.
//! A forgotten memory's text leaves the view the moment its key is destroyed: its events read
//! as erased from then on, and a cached copy is dropped when the forget is read.

use std::collections::{BTreeMap, BTreeSet};

use glave_event::{
    At, Body, EventId, MemoryForgotten, MemoryId, MemoryRestored, MemoryScope, MemorySource,
    Principal, PrincipalKind, RawEvent, decode,
};
use glave_ports::memories::Remembered;

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

/// What one project's memory ledger says.
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
    /// Pairs of memories tidy-up could not reconcile, waiting for the person: open until
    /// either memory changes.
    pub(crate) questions: Vec<[MemoryId; 2]>,
    /// When tidy-up last ran over this ledger.
    pub(crate) tidied: Option<At>,
    /// Where each memory the person (or the model for them) forgot was saved, and each memory
    /// tidy-up had merged into one of those: `recall` never returns those turns again.
    pub(crate) hidden: BTreeMap<MemoryId, MemorySource>,
    /// Memories tidy-up merged into another, until a restore undoes it.
    folded: BTreeMap<MemoryId, Folded>,
    /// Memories forgotten at once: by the person, or the model for them.
    gone: BTreeSet<MemoryId>,
}

/// A memory tidy-up merged into another.
#[derive(Debug, Clone, Copy)]
struct Folded {
    into: MemoryId,
    /// The version of `into` that took in its text: the merge's edit, which a restore may undo.
    edit: Option<EventId>,
    /// Where it was saved.
    source: Option<MemorySource>,
    /// The forget that merged it, which a restore names to undo it.
    event: EventId,
}

impl Book {
    pub(crate) fn next_seq(&self) -> u64 {
        self.next.max(1)
    }

    /// The memories in force, oldest first.
    pub(crate) fn live(&self) -> Vec<Remembered> {
        self.entries.values().filter_map(Entry::live).collect()
    }

    /// The open questions whose memories are both still in force.
    pub(crate) fn open_questions(&self) -> Vec<[MemoryId; 2]> {
        let live = |memory: &MemoryId| {
            self.entries
                .get(memory)
                .is_some_and(|entry| entry.forgot.is_none())
        };
        self.questions
            .iter()
            .filter(|pair| pair.iter().all(live))
            .copied()
            .collect()
    }

    /// Tidy-up ran at `at`, leaving `questions` for the person.
    fn tidied_at(&mut self, at: At, questions: Vec<[MemoryId; 2]>) {
        self.tidied = Some(at);
        for pair in questions {
            if !self.questions.contains(&pair) {
                self.questions.push(pair);
            }
        }
    }

    /// A change to `memory` answers any question about it.
    fn answered(&mut self, memory: MemoryId) {
        self.questions.retain(|pair| !pair.contains(&memory));
    }

    /// The turns `forgotten` (the event `event`, at `at`) takes out of `recall`. Glave's own
    /// forgets wait to be erased and take none: a memory merged away lives on in the one it went
    /// into, and one a later memory replaced, or past retention, is still history. A forget
    /// erased at once (the person's, or the model's for them) takes the turn the memory was saved
    /// in, and those of every memory merged into it, however many merges ago.
    fn hide(&mut self, forgotten: &MemoryForgotten, event: EventId, at: At) {
        if forgotten.erase_at > at {
            let Some(into) = forgotten.into else {
                return;
            };
            // The merge's edit of `into` comes first, so it is the version in force now.
            let edit = self
                .entries
                .get(&into)
                .and_then(Entry::current)
                .map(|(_, version)| version.event);
            let source = forgotten.source;
            self.folded.insert(
                forgotten.memory,
                Folded {
                    into,
                    edit,
                    source,
                    event,
                },
            );
            // Merged into a memory the person forgot meanwhile: hidden with it.
            if !self.gone.contains(&into) {
                return;
            }
        }
        self.gone.insert(forgotten.memory);
        if let Some(source) = forgotten.source {
            self.hidden.insert(forgotten.memory, source);
        }
        for (memory, source) in self.folded_into(forgotten.memory) {
            if let Some(source) = source {
                self.hidden.insert(memory, source);
            }
        }
    }

    /// The memories tidy-up merged into `memory`, however many merges ago, with where each was
    /// saved.
    fn folded_into(&self, memory: MemoryId) -> Vec<(MemoryId, Option<MemorySource>)> {
        let mut found = Vec::new();
        let mut seen = BTreeSet::from([memory]);
        let mut next = vec![memory];
        while let Some(into) = next.pop() {
            for (folded, merge) in &self.folded {
                if merge.into == into && seen.insert(*folded) {
                    found.push((*folded, merge.source));
                    next.push(*folded);
                }
            }
        }
        found
    }

    /// The memories merged into `memory` that still wait to be erased, through merges whose
    /// edits are still in force: one whose merged text was undone no longer lives on in it.
    pub(crate) fn merged_into(&self, memory: MemoryId) -> Vec<MemoryId> {
        let mut found = Vec::new();
        let mut next = vec![memory];
        while let Some(into) = next.pop() {
            for (merged, merge) in &self.folded {
                if merge.into != into || found.contains(merged) || !self.holds(merge) {
                    continue;
                }
                found.push(*merged);
                next.push(*merged);
            }
        }
        found.retain(|merged| {
            self.entries
                .get(merged)
                .is_some_and(|entry| entry.forgot.is_some())
        });
        found
    }

    /// Whether the memory `merge` went into still has the merge's text, not undone.
    fn holds(&self, merge: &Folded) -> bool {
        self.entries.get(&merge.into).is_some_and(|entry| {
            entry
                .versions
                .iter()
                .any(|version| Some(version.event) == merge.edit && !version.undone)
        })
    }

    /// The memories glave forgot that still wait to be erased, with their scopes.
    pub(crate) fn waiting(&self) -> Vec<(MemoryId, MemoryScope)> {
        self.entries
            .values()
            .filter(|entry| entry.forgot.is_some())
            .map(|entry| (entry.memory, entry.scope))
            .collect()
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
                self.answered(edited.memory);
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
                self.answered(forgotten.memory);
                self.hide(&forgotten, id, at);
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
            Body::MemoryRestored(restored) => self.restore(&restored),
            Body::MemoryTidied(tidied) => self.tidied_at(at, tidied.questions),
            _ => {}
        }
    }

    /// Undoes the change of glave's that `restored` names: a forget, a merge, or an edit.
    fn restore(&mut self, restored: &MemoryRestored) {
        self.answered(restored.memory);
        if self
            .folded
            .get(&restored.memory)
            .is_some_and(|merge| merge.event == restored.undone)
        {
            self.folded.remove(&restored.memory);
            self.gone.remove(&restored.memory);
        }
        if let Some(entry) = self.entries.get_mut(&restored.memory) {
            if entry
                .forgot
                .as_ref()
                .is_some_and(|forgot| forgot.event == restored.undone)
            {
                entry.forgot = None;
                self.hidden.remove(&restored.memory);
            }
            for version in entry.versions.iter_mut().skip(1) {
                if version.event == restored.undone {
                    version.undone = true;
                }
            }
        }
    }
}

/// Whether `by` is glave itself: its changes can be undone within 30 days.
pub(crate) fn by_glave(by: &Principal) -> bool {
    by.kind == PrincipalKind::Glave
}

#[cfg(test)]
mod tests {
    use glave_event::{PrincipalId, Seq, ThreadId};
    use uuid::Uuid;

    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(0x0192_f5c0_0000_7000_8000_0000_0000_0000 | n)
    }

    fn memory(n: u128) -> MemoryId {
        MemoryId::try_from(id(n)).unwrap()
    }

    fn day(n: u8) -> At {
        At::try_from(format!("2026-10-{n:02}T10:00:00Z")).unwrap()
    }

    /// Memory `n`, saved at `seq` `n` of one thread, forgotten on day 1: by glave (erased in
    /// 30 days, `into` another when merged), or by the person (erased at once).
    fn forget(book: &mut Book, n: u128, into: Option<u128>, by_person: bool) {
        let by = if by_person {
            Principal::person(PrincipalId::try_from(id(50)).unwrap())
        } else {
            Principal::glave()
        };
        let forgotten = MemoryForgotten {
            memory: memory(n),
            erase_at: if by_person { day(1) } else { day(31) },
            source: Some(MemorySource {
                thread: ThreadId::try_from(id(500)).unwrap(),
                seq: Seq::new(u64::try_from(n).unwrap()).unwrap(),
            }),
            into: into.map(memory),
            by,
        };
        let event = EventId::try_from(id(900 + n)).unwrap();
        book.hide(&forgotten, event, day(1));
    }

    fn hidden(book: &Book) -> Vec<MemoryId> {
        book.hidden.keys().copied().collect()
    }

    #[test]
    fn only_a_forget_erased_at_once_hides_turns_and_it_takes_every_merge_before_it() {
        let mut book = Book::default();
        forget(&mut book, 1, Some(2), false);
        forget(&mut book, 2, Some(3), false);
        forget(&mut book, 4, None, false);
        assert_eq!(
            hidden(&book),
            [],
            "glave's merges, contradictions and retention hide nothing"
        );
        forget(&mut book, 3, None, true);
        assert_eq!(
            hidden(&book),
            [memory(1), memory(2), memory(3)],
            "the memory forgotten, and the two merged into it one after the other"
        );
        forget(&mut book, 5, Some(3), false);
        assert_eq!(
            hidden(&book),
            [memory(1), memory(2), memory(3), memory(5)],
            "merged into it after the person forgot it, in a tidy-up already under way"
        );
    }
}
