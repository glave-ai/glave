//! The store's index of where schedules are, so every server that fires schedules fires
//! everyone's, though no one asked it for anything since it started. Each organization's own
//! stream lists its projects with schedules now (`schedule.placed`, `schedule.unplaced` as a
//! project's last one goes), so no organization's are listed under another's; the store says
//! which organizations it holds (`Projects::organizations`), a page at a time. Each
//! organization's index holds at most [`MOST_PROJECTS`] projects, each listed once; this server
//! keeps what it read of an organization's only while it lists any.

use std::collections::{HashMap, HashSet};
use std::sync::PoisonError;

use glave_event::{
    At, Body, EventId, Header, Payload, RawEvent, SchedulePlaced, ScheduleUnplaced, Seq, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up, in_order};
use glave_ports::{BlobStore, EventLog, Located, ModelProvider, OrgId, ProjectId, Scope};

use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most projects with schedules one organization's index holds at once: past it, a
/// schedule is refused in a project of theirs not listed yet. Each organization's own, so none
/// crowds out another's.
pub(crate) const MOST_PROJECTS: usize = 10_000;

/// The store's organizations one tick of the scheduler reads the indexes of, beyond those it
/// knows have schedules: where the tick before stopped, a page further on each time, so a
/// schedule made on another server in an organization this one never served is found.
pub(crate) const ORGANIZATIONS: usize = 1_000;

/// One organization's index, as far as this server has read it: its projects with schedules,
/// in the order they were listed.
#[derive(Debug, Default)]
struct Listed {
    next: u64,
    items: Vec<ProjectId>,
    held: HashSet<ProjectId>,
}

impl Listed {
    /// Takes in the event at the next `seq`.
    fn apply(&mut self, raw: RawEvent) {
        let Some((_, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::SchedulePlaced(placed) => {
                let project = ProjectId::from_uuid(placed.project);
                if self.items.len() < MOST_PROJECTS && self.held.insert(project) {
                    self.items.push(project);
                }
            }
            Body::ScheduleUnplaced(unplaced) => {
                let project = ProjectId::from_uuid(unplaced.project);
                if self.held.remove(&project) {
                    self.items.retain(|item| *item != project);
                }
            }
            _ => {}
        }
    }

    /// The `seq` the next event takes.
    fn next_seq(&self) -> u64 {
        self.next.max(1)
    }
}

/// The index as far as this server has read it, of organizations that list projects; how many
/// ticks of the scheduler read it, where in each organization's projects the last tick stopped,
/// and after which of the store's organizations the next page of them starts.
#[derive(Debug, Default)]
pub(crate) struct Placed {
    projects: HashMap<OrgId, Listed>,
    ticks: usize,
    stopped: HashMap<OrgId, usize>,
    after: Option<OrgId>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The organizations whose index, as this server last read it, lists projects.
    pub(crate) fn known_organizations(&self) -> Vec<OrgId> {
        let mut known: Vec<OrgId> = self.held_placed(|held| {
            (held.projects.iter())
                .filter(|(_, listed)| !listed.items.is_empty())
                .map(|(org, _)| *org)
                .collect()
        });
        known.sort();
        known
    }

    /// The next page of the store's organizations, after where the last one this server went
    /// through stopped; none while they can't be listed, tried again next time.
    pub(crate) async fn organizations_page(&self) -> Vec<OrgId> {
        let after = self.held_placed(|held| held.after);
        match self
            .inner
            .projects
            .organizations(after, ORGANIZATIONS)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(%error, "could not list the store's organizations");
                Vec::new()
            }
        }
    }

    /// Notes that the store's organizations were gone through up to `org`, so the next page
    /// starts after it; none starts again from the first.
    pub(crate) fn listed_through(&self, org: Option<OrgId>) {
        self.held_placed(|held| held.after = org);
    }

    /// The projects `org`'s index lists, read up to now; what can't be read now is what was
    /// read before, read again next time. One that lists none is kept no more, nor where a tick
    /// stopped in it, so organizations without schedules cost this server nothing.
    pub(crate) async fn placed_in(&self, org: OrgId) -> Vec<Located> {
        if let Err(error) = self.catch_up_placed(org).await {
            tracing::warn!(%error, %org, "could not read where the organization's schedules are");
        }
        let projects = self.held_placed(|held| {
            let items = (held.projects.get(&org)).map(|listed| listed.items.clone());
            if items.as_ref().is_none_or(Vec::is_empty) {
                held.projects.remove(&org);
                held.stopped.remove(&org);
            }
            items
        });
        (projects.into_iter().flatten())
            .map(|project| Located { org, project })
            .collect()
    }

    /// Every project the index lists, in every organization the store holds, read up to now,
    /// a page of organizations at a time: for a server as it starts.
    pub(crate) async fn placed(&self) -> Vec<Located> {
        let mut placed = Vec::new();
        let mut after = None;
        loop {
            let page = match self
                .inner
                .projects
                .organizations(after, ORGANIZATIONS)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(%error, "could not list the store's organizations");
                    break;
                }
            };
            for org in &page {
                placed.extend(self.placed_in(*org).await);
            }
            match page.last() {
                Some(last) if page.len() == ORGANIZATIONS => after = Some(*last),
                _ => break,
            }
        }
        placed
    }

    /// Which organization, counting from the first, this tick of the scheduler starts from:
    /// where the last one stopped, or one further on once it went round.
    pub(crate) fn next_turn(&self) -> usize {
        self.held_placed(|held| held.ticks)
    }

    /// Notes that this tick went through `reached` organizations from where it started, of
    /// `count`: the next starts where it stopped, or, having gone round, one further on.
    pub(crate) fn turned(&self, reached: usize, count: usize) {
        self.held_placed(|held| {
            let on = if reached >= count { 1 } else { reached };
            held.ticks = held.ticks.wrapping_add(on);
        });
    }

    /// Where in `org`'s `count` projects this tick of the scheduler starts: where the last one
    /// stopped.
    pub(crate) fn resume_at(&self, org: OrgId, count: usize) -> usize {
        let stopped = self.held_placed(|held| held.stopped.get(&org).copied());
        stopped.and_then(|at| at.checked_rem(count)).unwrap_or(0)
    }

    /// Notes where in `org`'s projects this tick stopped, for the next to go on from.
    pub(crate) fn stopped_at(&self, org: OrgId, at: usize) {
        self.held_placed(|held| {
            held.stopped.insert(org, at);
        });
    }

    /// Whether the project `at`'s organization's index lists it, read up to now: whether it has
    /// schedules now.
    pub(crate) async fn has_schedules(&self, at: Located) -> Result<bool, ApiError> {
        self.catch_up_placed(at.org).await?;
        Ok(self.held_placed(|held| {
            (held.projects.get(&at.org)).is_some_and(|listed| listed.held.contains(&at.project))
        }))
    }

    /// Lists the project `at` in its organization's index, unless it is listed already.
    /// Refused, in plain words, where its organization has as many projects with schedules as
    /// one may, so every server reads the same.
    pub(crate) async fn place_schedules(&self, at: Located) -> Result<(), ApiError> {
        let placed = SchedulePlaced {
            project: at.project.as_uuid(),
        };
        let absent = |held: &Placed| match held.projects.get(&at.org) {
            None => Some(1),
            Some(listed) if listed.held.contains(&at.project) => None,
            Some(listed) => (listed.items.len() < MOST_PROJECTS).then(|| listed.next_seq()),
        };
        self.list_once(at.org, &placed, absent).await?;
        let found = self.held_placed(|held| {
            (held.projects.get(&at.org)).is_some_and(|listed| listed.held.contains(&at.project))
        });
        if !found {
            return Err(ApiError::new(
                code::LIMIT,
                Sentence::TooManyProjectsScheduled {
                    most: MOST_PROJECTS,
                },
            ));
        }
        Ok(())
    }

    /// Takes the project `at` out of its organization's index once its ledger, read now, holds
    /// no schedule: what the index holds is the projects with schedules now. A schedule made
    /// there before the removal landed, which found the project still listed, is found as the
    /// ledger is read again after it, and lists the project again.
    pub(crate) async fn unplace_if_none(&self, at: Located) -> Result<(), ApiError> {
        if !self.ledger_apart(at).await?.schedules.is_empty() {
            return Ok(());
        }
        let unplaced = ScheduleUnplaced {
            project: at.project.as_uuid(),
        };
        let listed = |held: &Placed| {
            let listed = held.projects.get(&at.org)?;
            listed.held.contains(&at.project).then(|| listed.next_seq())
        };
        self.list_once(at.org, &unplaced, listed).await?;
        if self.ledger_apart(at).await?.schedules.is_empty() {
            return Ok(());
        }
        self.place_schedules(at).await
    }

    /// Appends `payload` to `org`'s index, after anything others wrote meanwhile, at the `seq`
    /// `due` gives of the index read up to then, or not at all when it gives none.
    async fn list_once<T: Payload>(
        &self,
        org: OrgId,
        payload: &T,
        due: impl Fn(&Placed) -> Option<u64>,
    ) -> Result<(), ApiError> {
        let (scope, index) = (Scope::of_organization(org), Reserved::Scheduled.id());
        let log = self.inner.harness.log();
        let due = &due;
        let appended = append_retrying(log, &scope, index, || async move {
            self.catch_up_placed(org).await?;
            let Some(next) = self.held_placed(|held| due(held)) else {
                return Ok::<_, ApiError>(None);
            };
            let now = self.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, self.inner.harness.entropy().bytes()?)?,
                thread: index,
                seq: Seq::new(next)?,
                at: At::from(now),
            };
            Ok(Some((vec![encode(header, payload)?], ())))
        })
        .await?;
        self.held_placed(|held| {
            let listed = held.projects.entry(org).or_default();
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                listed.apply(raw);
            }
        });
        Ok(())
    }

    /// Reads whatever was added to `org`'s index since this server last looked: glave's own
    /// stream, in that organization's own scope.
    async fn catch_up_placed(&self, org: OrgId) -> Result<(), ApiError> {
        let scope = Scope::of_organization(org);
        let next = self.held_placed(|held| (held.projects.get(&org)).map_or(1, Listed::next_seq));
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        catch_up(log, &scope, Reserved::Scheduled.id(), after, |page| {
            self.held_placed(|held| {
                let listed = held.projects.entry(org).or_default();
                for raw in page {
                    listed.apply(raw);
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
