//! The person's own tools, by where their threads use them (`specs/policy.md`, "Connections"): a
//! connection of theirs is used everywhere, or only in one project, their Chats among them. A
//! thread is offered those of its project alone, so a connection used elsewhere is neither
//! listed nor found in it, and a call to one its prefix still lists is refused (`toolset.rs`).
//! The connectors of glave's catalog they may connect and have not, `tool_search` finds for
//! them to connect.

use std::collections::BTreeMap;
use std::sync::Arc;

use glave_ports::{ProjectId, ToolSource};

/// The person's own tool sources for the threads of one place, and the names of the
/// connections among them: the connectors of glave's catalog the person connected.
#[derive(Clone, Default)]
pub struct Placed {
    pub sources: Vec<Arc<dyn ToolSource>>,
    pub connections: Vec<String>,
}

/// A connector of glave's catalog the person may connect and has not: what `tool_search`
/// finds of it, in English, for them to connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connectable {
    /// Its id in the catalog: `linear`.
    pub id: String,
    /// Its vendor's name for it: `Linear`.
    pub name: String,
    /// What it reaches: `Issues, projects and cycles.`
    pub description: String,
    /// The names of its categories and tags, which a search finds it by too.
    pub tags: Vec<String>,
    /// Whether connecting it signs the person in on its vendor's own page.
    pub signs_in: bool,
}

/// The person's own tool sources: those every place has, each place's in their stead where it
/// has connections of its own, the name of every connection they may connect, here or not, and
/// those of glave's catalog they may connect and have not.
#[derive(Clone, Default)]
pub struct OwnTools {
    everywhere: Placed,
    places: BTreeMap<ProjectId, Placed>,
    connectors: Vec<String>,
    connectable: Vec<Connectable>,
}

impl OwnTools {
    /// `sources`, none of them a connection, for the person's threads everywhere.
    pub fn everywhere(sources: Vec<Arc<dyn ToolSource>>) -> Self {
        Self::new(
            Placed {
                sources,
                connections: Vec::new(),
            },
            Vec::new(),
        )
    }

    /// `everywhere` for the threads of every place without connections of their own; any of
    /// `connectors` not among a place's connections is turned off there.
    pub fn new(everywhere: Placed, connectors: Vec<String>) -> Self {
        Self {
            everywhere,
            places: BTreeMap::new(),
            connectors,
            connectable: Vec::new(),
        }
    }

    /// With `placed` for the threads of `project` alone, in place of those everywhere: the
    /// project's own connections, beside those of everywhere it keeps.
    #[must_use]
    pub fn with_place(mut self, project: ProjectId, placed: Placed) -> Self {
        self.places.insert(project, placed);
        self
    }

    /// With `connectable`, the connectors of glave's catalog the person may connect and has
    /// not, for `tool_search` to find in every place.
    #[must_use]
    pub fn with_connectable(mut self, connectable: Vec<Connectable>) -> Self {
        self.connectable = connectable;
        self
    }

    /// The person's own for the threads of `project`.
    pub(crate) fn of(&self, project: ProjectId) -> Mine {
        let placed = self.places.get(&project).unwrap_or(&self.everywhere);
        let away = (self.connectors.iter())
            .filter(|name| !placed.connections.contains(name))
            .cloned()
            .collect();
        Mine {
            placed: placed.clone(),
            away,
            connectable: self.connectable.clone(),
        }
    }
}

/// The person's own for the threads of one place: its sources, the connections the person
/// may connect that are not among them, and the connectors of glave's catalog they may connect
/// and have not.
#[derive(Clone, Default)]
pub(crate) struct Mine {
    pub(crate) placed: Placed,
    pub(crate) away: Vec<String>,
    pub(crate) connectable: Vec<Connectable>,
}
