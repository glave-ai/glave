//! The store's index of where schedules are (`schedule.placed`): one stream in the server's own
//! organization, listing each project someone made a schedule in, once. Every server that fires
//! schedules reads it, so it fires everyone's, though no one asked it for anything since it
//! started.

use std::collections::HashSet;
use std::sync::PoisonError;

use glave_event::{At, Body, EventId, Header, RawEvent, SchedulePlaced, Seq, encode};
use glave_ports::stream::{Reserved, append_retrying, catch_up, in_order};
use glave_ports::{BlobStore, EventLog, Located, ModelProvider, OrgId, ProjectId, Scope};

use crate::protocol::ApiError;
use crate::server::Server;

/// The most projects a server reads from the index: past it, a project's schedules fire once
/// its person asks this server for anything, as before the index.
const MOST_PLACED: usize = 100_000;

/// The index as far as this server has read it.
#[derive(Debug, Default)]
pub(crate) struct Placed {
    next: u64,
    /// In the order they were placed.
    projects: Vec<Located>,
    listed: HashSet<Located>,
}

impl Placed {
    fn apply(&mut self, raw: RawEvent) {
        let Some((_, Body::SchedulePlaced(placed))) = in_order(&mut self.next, raw) else {
            return;
        };
        let at = Located {
            org: OrgId::from_uuid(placed.org),
            project: ProjectId::from_uuid(placed.project),
        };
        if self.projects.len() < MOST_PLACED && self.listed.insert(at) {
            self.projects.push(at);
        }
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Every project the index lists, read up to now.
    pub(crate) async fn placed(&self) -> Result<Vec<Located>, ApiError> {
        self.catch_up_placed().await?;
        Ok(self.held_placed(|held| held.projects.clone()))
    }

    /// Whether the index lists the project `at`, read up to now: whether anyone made a
    /// schedule there.
    pub(crate) async fn has_schedules(&self, at: Located) -> Result<bool, ApiError> {
        self.catch_up_placed().await?;
        Ok(self.held_placed(|held| held.listed.contains(&at)))
    }

    /// Lists the project `at` in the index, unless it is listed already.
    pub(crate) async fn place_schedules(&self, at: Located) -> Result<(), ApiError> {
        // Glave's own: the store's index, in the server's own organization.
        let scope = Scope::of_organization(self.inner.own);
        let index = Reserved::Scheduled.id();
        let log = self.inner.harness.log();
        let appended = append_retrying(log, &scope, index, || async move {
            self.catch_up_placed().await?;
            let Some(next) =
                self.held_placed(|held| (!held.listed.contains(&at)).then_some(held.next))
            else {
                return Ok::<_, ApiError>(None);
            };
            let now = self.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, self.inner.harness.entropy().bytes()?)?,
                thread: index,
                seq: Seq::new(next.max(1))?,
                at: At::from(now),
            };
            let placed = SchedulePlaced {
                org: at.org.as_uuid(),
                project: at.project.as_uuid(),
            };
            Ok(Some((vec![encode(header, &placed)?], ())))
        })
        .await?;
        self.held_placed(|held| {
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                held.apply(raw);
            }
        });
        Ok(())
    }

    /// Reads whatever was added to the index since this server last looked.
    async fn catch_up_placed(&self) -> Result<(), ApiError> {
        // Glave's own: the store's index, in the server's own organization.
        let scope = Scope::of_organization(self.inner.own);
        let next = self.held_placed(|held| held.next.max(1));
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::Scheduled.id(), after, |page| {
            self.held_placed(|held| {
                for raw in page {
                    held.apply(raw);
                }
            });
        })
        .await?;
        Ok(())
    }

    fn held_placed<R>(&self, look: impl FnOnce(&mut Placed) -> R) -> R {
        let mut held = (self.inner.placed)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(&mut held)
    }
}
