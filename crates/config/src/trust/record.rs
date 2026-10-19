//! What the person has decided, as the trust ledger in the store holds it: one entry per
//! project, found by its canonical path or, for a clone that moved, by its `origin` remote.
//! Each entry is a `trust.decided` event, the latest about a project replacing the ones before.
//! What is trusted is the content (each server's definition), not just a name, so a repository
//! that changes a server's command asks again.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

pub use glave_event::{Decision, TrustDecided as Entry};

use crate::trust::project::{Item, Project};

/// The file an earlier glave kept decisions in, in its folder: read once to move them into the
/// store, then deleted.
pub const FILE: &str = "trust.toml";

/// The only version of that file there was.
const VERSION: u32 = 1;

/// Every decision, each project's latest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    project: Vec<Entry>,
}

/// The file an earlier glave kept.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Earlier {
    version: u32,
    #[serde(default)]
    project: Vec<Entry>,
}

/// The decision `trusted` about `item`.
pub fn decision(item: &Item, trusted: bool) -> Decision {
    Decision {
        definition: item.definition.clone(),
        trusted,
    }
}

impl Record {
    /// The decisions in `text`, the file at `path` an earlier glave kept.
    pub fn earlier(text: &str, path: &Path) -> crate::Result<Vec<Entry>> {
        let earlier: Earlier = toml::from_str(text)
            .map_err(|error| crate::Error::Invalid(format!("{}: {error}", path.display())))?;
        if earlier.version != VERSION {
            crate::error::invalid!(
                "{}: version = {} is not one this glave knows: use version = {VERSION}",
                path.display(),
                earlier.version
            );
        }
        Ok(earlier.project)
    }

    /// Every project's latest decisions.
    pub fn entries(&self) -> &[Entry] {
        &self.project
    }

    /// The decisions about `project`: found by path, else by the remote a moved clone shares.
    pub fn entry_for(&self, project: &Project) -> Option<&Entry> {
        self.find(&project.root.to_string_lossy(), project.remote.as_deref())
    }

    fn find(&self, path: &str, remote: Option<&str>) -> Option<&Entry> {
        self.project
            .iter()
            .find(|entry| entry.path == path)
            .or_else(|| {
                let remote = remote?;
                self.project
                    .iter()
                    .find(|entry| entry.remote.as_deref() == Some(remote))
            })
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

    /// Takes in `entry`, replacing what was decided about its project: the entry its path or,
    /// failing that, its remote finds. One with no items forgets the project.
    pub fn take(&mut self, entry: Entry) {
        let known = self
            .find(&entry.path, entry.remote.as_deref())
            .map(|known| known.path.clone());
        self.project
            .retain(|kept| Some(&kept.path) != known.as_ref());
        if !entry.items.is_empty() {
            self.project.push(entry);
        }
    }
}
