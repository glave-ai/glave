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
    /// Whether its name is also a plain English word (Make, Close, Square), so a search that
    /// says it may not mean the service.
    pub plain_word: bool,
}

/// glave's catalog as it is offered to the person: the connectors of it they may connect and
/// have not, none where they connected every one, and the words a quarter or more of the whole
/// catalog's descriptions and tags have, as a search reads them, which say nothing of any one
/// service (`and`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Offered {
    pub connectable: Vec<Connectable>,
    pub common: Vec<String>,
}

/// The person's own tool sources: those every place has, each place's in their stead where it
/// has connections of its own, the name of every connection they may connect, here or not, and
/// glave's catalog, where it is offered to them.
#[derive(Clone, Default)]
pub struct OwnTools {
    everywhere: Placed,
    places: BTreeMap<ProjectId, Placed>,
    connectors: Vec<String>,
    offered: Option<Offered>,
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
            offered: None,
        }
    }

    /// With `placed` for the threads of `project` alone, in place of those everywhere: the
    /// project's own connections, beside those of everywhere it keeps.
    #[must_use]
    pub fn with_place(mut self, project: ProjectId, placed: Placed) -> Self {
        self.places.insert(project, placed);
        self
    }

    /// With glave's catalog offered to the person as `offered` says, even with none of it left
    /// to connect: `tool_search` is listed to find what is, in every place, from each thread's
    /// first call.
    #[must_use]
    pub fn with_catalog_offered(mut self, offered: Offered) -> Self {
        self.offered = Some(offered);
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
            offered: self.offered.clone(),
        }
    }
}

/// The person's own for the threads of one place: its sources, the connections the person
/// may connect that are not among them, and glave's catalog, where it is offered to them.
#[derive(Clone, Default)]
pub(crate) struct Mine {
    pub(crate) placed: Placed,
    pub(crate) away: Vec<String>,
    pub(crate) offered: Option<Offered>,
}
