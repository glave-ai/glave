//! What the person has decided, kept in `trust.toml` in glave's folder. One entry per project,
//! found by its canonical path or, for a clone that moved, by its `origin` remote. What is
//! trusted is the content (each server's definition), not just a name, so a repository that
//! changes a server's command asks again.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::trust::project::{Item, Project};

/// The file, in glave's folder.
pub const FILE: &str = "trust.toml";

/// The only version of the file there is.
const VERSION: u32 = 1;

/// The person's decision on one item, and exactly what it was about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub definition: String,
    pub trusted: bool,
}

impl Decision {
    /// The decision `trusted` about `item`.
    pub fn about(item: &Item, trusted: bool) -> Self {
        Self {
            definition: item.definition.clone(),
            trusted,
        }
    }
}

/// One project's decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(default)]
    pub items: BTreeMap<String, Decision>,
}

/// Every decision, as `trust.toml` holds them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    version: u32,
    #[serde(default)]
    project: Vec<Entry>,
}

impl Default for Record {
    fn default() -> Self {
        Self {
            version: VERSION,
            project: Vec::new(),
        }
    }
}

impl Record {
    /// The record in `home`; empty when there is none yet.
    pub async fn load(home: &Path) -> crate::Result<Self> {
        let path = home.join(FILE);
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(crate::Error::io("read", path)(error));
            }
        };
        let record: Self = toml::from_str(&text)
            .map_err(|error| crate::Error::Invalid(format!("{}: {error}", path.display())))?;
        if record.version != VERSION {
            crate::error::invalid!(
                "{}: version = {} is not one this glave knows: use version = {VERSION}",
                path.display(),
                record.version
            );
        }
        Ok(record)
    }

    /// Writes the record to `home`, whole and then moved into place so a crash never leaves
    /// half of it.
    pub async fn save(&self, home: &Path) -> crate::Result<()> {
        let text = toml::to_string(self)
            .map_err(|error| crate::Error::within("could not write the trust record", error))?;
        tokio::fs::create_dir_all(home)
            .await
            .map_err(crate::Error::io("create", home))?;
        let path = home.join(FILE);
        // A name of this process's own, so two glaves saving at once never write one file.
        let temporary = home.join(format!("{FILE}.{}.new", std::process::id()));
        tokio::fs::write(&temporary, text)
            .await
            .map_err(crate::Error::io("write", &temporary))?;
        tokio::fs::rename(&temporary, &path)
            .await
            .map_err(crate::Error::io("replace", path))
    }

    /// The decisions about `project`: found by path, else by the remote a moved clone shares.
    pub fn entry_for(&self, project: &Project) -> Option<&Entry> {
        let path = project.root.to_string_lossy();
        self.project
            .iter()
            .find(|entry| entry.path == path)
            .or_else(|| {
                let remote = project.remote.as_deref()?;
                self.project
                    .iter()
                    .find(|entry| entry.remote.as_deref() == Some(remote))
            })
    }

    /// Sets what is decided about `project` to exactly `decisions`, so an item the project no
    /// longer brings is forgotten.
    pub fn remember(&mut self, project: &Project, decisions: BTreeMap<String, Decision>) {
        let known = self.entry_for(project).map(|entry| entry.path.clone());
        self.project
            .retain(|entry| Some(&entry.path) != known.as_ref());
        if decisions.is_empty() {
            return;
        }
        self.project.push(Entry {
            path: project.root.to_string_lossy().into_owned(),
            remote: project.remote.clone(),
            items: decisions,
        });
    }
}
