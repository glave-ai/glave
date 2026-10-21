//! One memory ledger, read from its events as stored: what is in force, what was forgotten and
//! by whom, and how often tidy-up ran.

use std::collections::BTreeMap;

use glave_event::{Body, MemoryId, MemoryScope, PrincipalKind, RawEvent, decode};

/// What one memory ledger says.
#[derive(Default)]
pub(super) struct Ledger {
    /// Each memory saved, with its scope and its latest text.
    texts: BTreeMap<MemoryId, (MemoryScope, String)>,
    /// Every memory forgotten and not restored.
    pub(super) forgotten: Vec<MemoryId>,
    /// Those of them glave forgot, in tidy-up.
    by_glave: Vec<MemoryId>,
    /// Tidy-up's runs.
    pub(super) tidied: u32,
}

impl Ledger {
    pub(super) fn read(raw: Vec<RawEvent>) -> Self {
        let mut ledger = Self::default();
        for event in raw {
            // A memory whose key was destroyed reads as erased: it does not decode.
            match decode(event).map(|event| event.body) {
                Ok(Body::MemorySaved(saved)) => {
                    ledger.texts.insert(saved.memory, (saved.scope, saved.text));
                }
                Ok(Body::MemoryEdited(edited)) => {
                    if let Some(entry) = ledger.texts.get_mut(&edited.memory) {
                        entry.1 = edited.text;
                    }
                }
                Ok(Body::MemoryForgotten(gone)) => {
                    if gone.by.kind == PrincipalKind::Glave {
                        ledger.by_glave.push(gone.memory);
                    }
                    ledger.forgotten.push(gone.memory);
                }
                Ok(Body::MemoryRestored(restored)) => {
                    ledger.forgotten.retain(|memory| *memory != restored.memory);
                    ledger.by_glave.retain(|memory| *memory != restored.memory);
                }
                Ok(Body::MemoryTidied(_)) => ledger.tidied = ledger.tidied.saturating_add(1),
                _ => {}
            }
        }
        ledger
    }

    /// The memories in force, with their scope.
    pub(super) fn live(&self) -> impl Iterator<Item = (MemoryScope, &String)> {
        self.texts
            .iter()
            .filter(|(memory, _)| !self.forgotten.contains(memory))
            .map(|(_, (kind, text))| (*kind, text))
    }

    /// Forgotten memories whose text can still be opened.
    pub(super) fn readable_forgotten(&self) -> impl Iterator<Item = MemoryId> {
        self.forgotten
            .iter()
            .filter(|memory| self.texts.contains_key(memory))
            .copied()
    }

    /// The text of each memory glave forgot.
    pub(super) fn by_glave(&self) -> impl Iterator<Item = String> {
        self.by_glave
            .iter()
            .filter_map(|memory| self.texts.get(memory))
            .map(|(_, text)| text.clone())
    }
}
