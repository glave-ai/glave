//! Where schedules are on the store, so the server firing schedules fires everyone's, though no
//! one asked it anything since it started: one stream of glave's own beside threads, in the
//! server's own organization's scope, records a project placed as its first schedule is made
//! (`schedule.placed`) and unplaced as its last goes (`schedule.unplaced`), each naming its
//! organization and project, ids alone. It is read from where this server last read it, each
//! tick and as anything is placed, and from its start once; what this server keeps of it is the
//! projects with schedules now, as many as people keep schedules, never one for each
//! organization the store holds. A person's schedules are made and unplaced one at a time, on
//! any server (`Leased::Schedules`), so a project is never unplaced while a schedule is made in
//! it.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::PoisonError;

use glave_event::{
    At, Body, EventId, Header, Payload, PrincipalId, RawEvent, SchedulePlaced, ScheduleUnplaced,
    Seq, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up, in_order};
use glave_ports::{BlobStore, EventLog, Lease, Leased, Located, ModelProvider, OrgId, ProjectId};

use crate::protocol::ApiError;
use crate::server::Server;
use crate::words::Sentence;

/// The stream as far as this server has read it: the projects with schedules, in the order
/// they were placed; how many ticks of the scheduler read it, where in each organization's
/// projects the last tick stopped, kept only for organizations with projects listed, and whose
/// schedules a request here holds now.
#[derive(Debug, Default)]
pub(crate) struct Placed {
    next: u64,
    items: Vec<Located>,
    held: HashSet<Located>,
    ticks: usize,
    stopped: HashMap<OrgId, usize>,
    making: HashSet<(OrgId, PrincipalId)>,
}

/// A person's schedules, held by one request of this server's: let go with
/// [`Server::let_go_schedules`], or, dropped as a request is cancelled, here at once and on the
/// store as the lease lapses.
pub(crate) struct HeldSchedules<P, L, B> {
    server: Server<P, L, B>,
    lease: Option<Lease>,
    whose: (OrgId, PrincipalId),
}

impl<P, L, B> Drop for HeldSchedules<P, L, B> {
    fn drop(&mut self) {
        let mut held = (self.server.inner.placed)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        held.making.remove(&self.whose);
    }
}

impl Placed {
    /// Takes in the event at the next `seq`.
    fn apply(&mut self, raw: RawEvent) {
        let Some((_, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::SchedulePlaced(placed) => {
                let at = located(placed.org, placed.project);
                if self.held.insert(at) {
                    self.items.push(at);
                }
            }
            Body::ScheduleUnplaced(unplaced) => {
                let at = located(unplaced.org, unplaced.project);
                if self.held.remove(&at) {
                    self.items.retain(|item| *item != at);
                    if !self.items.iter().any(|item| item.org == at.org) {
                        self.stopped.remove(&at.org);
                    }
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

/// The project `project` of the organization `org`, as the stream names them.
fn located(org: uuid::Uuid, project: uuid::Uuid) -> Located {
    Located {
        org: OrgId::from_uuid(org),
        project: ProjectId::from_uuid(project),
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Every project with schedules, the stream read up to now; what can't be read now is what
    /// was read before, read again next time.
    pub(crate) async fn placed(&self) -> Vec<Located> {
        if let Err(error) = self.catch_up_placed().await {
            tracing::warn!(%error, "could not read where schedules are");
        }
        self.held_placed(|held| held.items.clone())
    }

    /// The organizations with projects listed, of `placed`, and `mine`'s: each one the tick
    /// goes through; where a tick stopped is kept for these alone.
    pub(crate) fn tick_organizations(
        &self,
        placed: &[Located],
        mine: &[Located],
    ) -> BTreeSet<OrgId> {
        let organizations: BTreeSet<OrgId> = placed.iter().chain(mine).map(|at| at.org).collect();
        self.held_placed(|held| held.stopped.retain(|org, _| organizations.contains(org)));
        organizations
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

    /// Takes `person`'s schedules in `org` for one request of this server's, so making one and
    /// unplacing a project are one step at a time, here and on any server: `None` while another
    /// request here, or another server, holds them.
    pub(crate) async fn take_schedules(
        &self,
        org: OrgId,
        person: PrincipalId,
    ) -> Result<Option<HeldSchedules<P, L, B>>, ApiError> {
        let whose = (org, person);
        if !self.held_placed(|held| held.making.insert(whose)) {
            return Ok(None);
        }
        let mut schedules = HeldSchedules {
            server: self.clone(),
            lease: None,
            whose,
        };
        schedules.lease = self.take_on(org, Leased::Schedules(person)).await?;
        Ok(schedules.lease.is_some().then_some(schedules))
    }

    /// Lets go of a person's schedules a request here held.
    pub(crate) async fn let_go_schedules(&self, mut schedules: HeldSchedules<P, L, B>) {
        if let Some(lease) = schedules.lease.take() {
            self.let_go(&lease).await;
        }
    }

    /// Lists the project `at` where schedules are, unless it is listed already, for its
    /// owner, who holds their schedules (`take_schedules`).
    pub(crate) async fn place_schedules(&self, at: Located) -> Result<(), ApiError> {
        let placed = SchedulePlaced {
            org: at.org.as_uuid(),
            project: at.project.as_uuid(),
        };
        let absent = |held: &Placed| (!held.held.contains(&at)).then(|| held.next_seq());
        self.list_once(&placed, absent).await
    }

    /// Takes the project `at` off where schedules are once its ledger, read now, holds no
    /// schedule, for `owner`, whose schedules it holds the while; while another server holds
    /// them, as it makes one, the project stays listed, which costs only its ledger read each
    /// tick.
    pub(crate) async fn unplace_if_none(
        &self,
        at: Located,
        owner: PrincipalId,
    ) -> Result<(), ApiError> {
        let Some(schedules) = self.take_schedules(at.org, owner).await? else {
            return Ok(());
        };
        let unplaced = async {
            if !self.ledger_apart(at).await?.schedules.is_empty() {
                return Ok(());
            }
            let gone = ScheduleUnplaced {
                org: at.org.as_uuid(),
                project: at.project.as_uuid(),
            };
            let listed = |held: &Placed| held.held.contains(&at).then(|| held.next_seq());
            self.list_once(&gone, listed).await
        };
        let done = unplaced.await;
        self.let_go_schedules(schedules).await;
        done
    }

    /// Appends `payload` to the stream, after anything others wrote meanwhile, at the `seq`
    /// `due` gives of the stream read up to then, or not at all when it gives none.
    async fn list_once<T: Payload>(
        &self,
        payload: &T,
        due: impl Fn(&Placed) -> Option<u64>,
    ) -> Result<(), ApiError> {
        let (scope, stream) = (self.placed_scope(), Reserved::Scheduled.id());
        let log = self.inner.harness.log();
        let (due, scope_ref) = (&due, &scope);
        let appended = append_retrying(log, scope_ref, stream, || async move {
            self.catch_up_placed().await?;
            let Some(next) = self.held_placed(|held| due(held)) else {
                return Ok::<_, ApiError>(None);
            };
            let now = self.inner.harness.clock().now();
            let header = Header {
                id: EventId::new(now, self.inner.harness.entropy().bytes()?)?,
                thread: stream,
                seq: Seq::new(next)?,
                at: At::from(now),
            };
            Ok(Some((vec![encode(header, payload)?], ())))
        })
        .await?;
        self.held_placed(|held| {
            for raw in appended.into_iter().flat_map(|(events, ())| events) {
                held.apply(raw);
            }
        });
        Ok(())
    }

    /// Reads whatever was added to the stream since this server last looked.
    async fn catch_up_placed(&self) -> Result<(), ApiError> {
        let next = self.held_placed(|held| held.next_seq());
        let after = Seq::new(next.saturating_sub(1)).ok();
        let log = self.inner.harness.log();
        let scope = self.placed_scope();
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

    /// Where the stream is: the server's own organization's scope, glave's own, every
    /// organization's ids in it and nothing more, read by the scheduler alone.
    fn placed_scope(&self) -> glave_ports::Scope {
        glave_ports::Scope::of_organization(self.inner.own)
    }

    fn held_placed<R>(&self, look: impl FnOnce(&mut Placed) -> R) -> R {
        let mut held = (self.inner.placed)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(&mut held)
    }
}

/// Refused while another server makes or unplaces a schedule of the same person's: they try
/// again.
pub(crate) fn schedules_busy() -> ApiError {
    ApiError::new(crate::protocol::code::INTERNAL, Sentence::WrittenElsewhere)
}
