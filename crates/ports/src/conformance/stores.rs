//! Every suite a whole store passes, one at a time, for a store a test made its own way: one
//! carried forward from an earlier schema by its migrations (`specs/storage.md`, "Migrations").

use super::Verdict;
use crate::directory::Directory;
use crate::leases::Leases;
use crate::log::EventLog;
use crate::projects::Projects;

/// A suite a whole store passes, each run on a store of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreSuite {
    EventLog,
    Search,
    ThreadViews,
    Projects,
    Directory,
    Leases,
}

impl StoreSuite {
    pub const ALL: [Self; 6] = [
        Self::EventLog,
        Self::Search,
        Self::ThreadViews,
        Self::Projects,
        Self::Directory,
        Self::Leases,
    ];
}

/// Runs `suite` on a store's log, projects, directory and leases.
pub async fn store_suite<L: EventLog>(
    suite: StoreSuite,
    log: &L,
    projects: &dyn Projects,
    directory: &dyn Directory,
    leases: &dyn Leases,
) -> Verdict {
    match suite {
        StoreSuite::EventLog => super::event_log(log).await,
        StoreSuite::Search => super::search(log).await,
        StoreSuite::ThreadViews => super::thread_views(log).await,
        StoreSuite::Projects => super::projects(projects).await,
        StoreSuite::Directory => super::directory(directory).await,
        StoreSuite::Leases => super::leases(leases).await,
    }
}
