//! What the person has decided, as the trust ledger in the store holds it: one entry per
//! project, found by its canonical path alone, so a moved folder is asked about again. Each
//! entry is a `trust.decided` event, the latest about a project replacing the ones before.
//! What is trusted is the content (each server's definition), not just a name, so a repository
//! that changes a server's command asks again.

pub use glave_event::{Decision, TrustDecided as Entry};
use std::collections::BTreeMap;

use crate::trust::project::{Item, Project};

/// Every decision, each project's latest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    project: Vec<Entry>,
}

/// The decision `trusted` about `item`.
pub(super) fn decision(item: &Item, trusted: bool) -> Decision {
    Decision {
        definition: item.definition.clone(),
        trusted,
    }
}

impl Record {
    /// Every project's latest decisions.
    pub fn entries(&self) -> &[Entry] {
        &self.project
    }

    /// The decisions about `project`, found by its path alone: a remote is only what a folder's
    /// own files say, so any folder could name a trusted one's.
    pub fn entry_for(&self, project: &Project) -> Option<&Entry> {
        self.find(&project.root.to_string_lossy())
    }

    fn find(&self, path: &str) -> Option<&Entry> {
        self.project.iter().find(|entry| entry.path == path)
    }

    /// What is decided about `project` set to exactly `decisions`, so an item the project no
    /// longer brings is forgotten: the entry to keep.
    pub fn deciding(project: &Project, decisions: BTreeMap<String, Decision>) -> Entry {
        Entry {
            path: project.root.to_string_lossy().into_owned(),
            remote: project.remote.clone(),
            items: decisions,
            by: None,
        }
    }

    /// Takes in `entry`, replacing what was decided about the project at its path. One with no
    /// items forgets the project.
    pub fn take(&mut self, entry: Entry) {
        self.project.retain(|kept| kept.path != entry.path);
        if !entry.items.is_empty() {
            self.project.push(entry);
        }
    }
}
