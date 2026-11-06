//! The person's own tools, by where their threads use them (`specs/policy.md`, "Connections"): a
//! connection of theirs is used everywhere, or only in one project, their Chats among them. A
//! thread is offered those of its project alone, so a connection used elsewhere is neither
//! listed nor found in it, and a call to one its prefix still lists is refused (`toolset.rs`).

use std::collections::BTreeMap;
use std::sync::Arc;

use glave_ports::{ProjectId, ToolSource};

/// The person's own tool sources: those every place has, and each place's in their stead where
/// it has connections of its own.
#[derive(Clone, Default)]
pub struct OwnTools {
    everywhere: Vec<Arc<dyn ToolSource>>,
    places: BTreeMap<ProjectId, Vec<Arc<dyn ToolSource>>>,
}

impl OwnTools {
    /// `sources`, for the person's threads everywhere.
    pub fn everywhere(sources: Vec<Arc<dyn ToolSource>>) -> Self {
        Self {
            everywhere: sources,
            places: BTreeMap::new(),
        }
    }

    /// With `sources` for the threads of `project` alone, in place of those everywhere: the
    /// project's own connections, beside those of everywhere it keeps.
    #[must_use]
    pub fn with_place(mut self, project: ProjectId, sources: Vec<Arc<dyn ToolSource>>) -> Self {
        self.places.insert(project, sources);
        self
    }

    /// The sources of the threads of `project`.
    pub(crate) fn of(&self, project: ProjectId) -> &[Arc<dyn ToolSource>] {
        self.places.get(&project).unwrap_or(&self.everywhere)
    }
}
