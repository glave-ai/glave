//! The person's own tools, by where their threads use them (`specs/policy.md`, "Connections"): a
//! connection of theirs is used everywhere, or only in one project, their Chats among them. A
//! thread is offered those of its project alone, so a connection used elsewhere is neither
//! listed nor found in it, and a call to one its prefix still lists is refused (`toolset.rs`).

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

/// The person's own tool sources: those every place has, each place's in their stead where it
/// has connections of its own, and the name of every connection they may connect, here or not.
#[derive(Clone, Default)]
pub struct OwnTools {
    everywhere: Placed,
    places: BTreeMap<ProjectId, Placed>,
    connectors: Vec<String>,
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
        }
    }

    /// With `placed` for the threads of `project` alone, in place of those everywhere: the
    /// project's own connections, beside those of everywhere it keeps.
    #[must_use]
    pub fn with_place(mut self, project: ProjectId, placed: Placed) -> Self {
        self.places.insert(project, placed);
        self
    }

    /// The sources of the threads of `project`, and the connections the person may connect
    /// that are not among them.
    pub(crate) fn of(&self, project: ProjectId) -> (Placed, Vec<String>) {
        let placed = self.places.get(&project).unwrap_or(&self.everywhere);
        let away = (self.connectors.iter())
            .filter(|name| !placed.connections.contains(name))
            .cloned()
            .collect();
        (placed.clone(), away)
    }
}
