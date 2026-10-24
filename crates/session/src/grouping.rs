//! How the caller's clients group their threads (`specs/session.md`, "Watching a list"): by
//! project until they choose, kept in their settings ledger as `grouping.set`, so every client
//! of theirs, on every server on one store, groups them the same. `grouping/get` and
//! `grouping/set` read and keep their choice. The server only keeps it: what each thread is
//! doing is its word, and a client groups by it.

use glave_event::{Body, Grouping, GroupingSet, Payload as _};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::protocol::{ApiError, Reply};
use crate::server::Server;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `grouping/get`: the caller's choice, else by project.
    pub(crate) async fn grouping(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let grouping = match self.setting(caller, GroupingSet::TYPE).await? {
            Some(Body::GroupingSet(set)) => set.grouping,
            _ => Grouping::default(),
        };
        Ok(Reply::Grouping { grouping })
    }

    /// Answers `grouping/set`: keeps `grouping` as the caller's.
    pub(crate) async fn set_grouping(
        &self,
        caller: &Caller,
        grouping: Grouping,
    ) -> Result<Reply, ApiError> {
        self.keep_setting(caller, &GroupingSet { grouping }).await?;
        Ok(Reply::Grouping { grouping })
    }
}
