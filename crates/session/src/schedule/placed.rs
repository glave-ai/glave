//! Where schedules are on the store, so the server firing schedules fires everyone's, though no
//! one asked it anything since it started: one stream of glave's own beside threads, in the
//! server's own organization's scope, records a project placed as a schedule is made there
//! (`schedule.placed`) and unplaced (`schedule.unplaced`), each naming its organization and
//! project, ids alone. It is read from where this server last read it, each tick and as anything
//! is placed, and from its start once; what this server keeps of it is the projects with
//! schedules now, as many as people keep schedules, never one for each organization the store
//! holds. Requests only ever place a project; the scheduler alone unplaces one, once its ledger
//! has held no schedule for a day, and places it back at once if a schedule came meanwhile, so
//! no order of steps loses a schedule.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::sync::PoisonError;

use glave_event::{
    At, Body, EventId, Header, Payload, RawEvent, SchedulePlaced, ScheduleUnplaced, Seq, encode,
};
use glave_ports::stream::{Reserved, append_retrying, catch_up, in_order};
use glave_ports::{BlobStore, EventLog, Located, ModelProvider, OrgId, ProjectId, Scope};
use jiff::SignedDuration;

use crate::protocol::ApiError;
use crate::server::Server;

/// How long a project's ledger holds no schedule before the scheduler unplaces it: a day, so a
/// project whose schedules come and go costs at most one pair of events a day.
const IDLE: SignedDuration = SignedDuration::from_hours(24);

/// The stream as far as this server has read it: the projects with schedules, in the order
/// they were placed, each with when; how many ticks of the scheduler read it, and where in each
/// organization's projects the last tick stopped, kept only for organizations it goes through.
#[derive(Debug, Default)]
pub(crate) struct Placed {
    next: u64,
    items: Vec<Located>,
    held: HashMap<Located, At>,
    ticks: usize,
    stopped: HashMap<OrgId, usize>,
}

impl Placed {
    /// Takes in the event at the next `seq`.
    fn apply(&mut self, raw: RawEvent) {
        let Some((at, body)) = in_order(&mut self.next, raw) else {
            return;
        };
        match body {
            Body::SchedulePlaced(placed) => {
                let project = located(placed.org, placed.project);
                if let Entry::Vacant(vacant) = self.held.entry(project) {
                    vacant.insert(at);
                    self.items.push(project);
                }
            }
            Body::ScheduleUnplaced(unplaced) => {
                let project = located(unplaced.org, unplaced.project);
                if self.held.remove(&project).is_some() {
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

    /// Whether the stream, as last read, lists the project `at`.
    pub(crate) fn is_placed(&self, at: Located) -> bool {
        self.held_placed(|held| held.held.contains_key(&at))
    }

    /// Keeps where a tick stopped for `organizations` alone, those the tick goes through.
    pub(crate) fn tick_organizations(&self, organizations: &BTreeSet<OrgId>) {
        self.held_placed(|held| held.stopped.retain(|org, _| organizations.contains(org)));
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

    /// Lists the project `at` where schedules are, unless it is listed already.
    pub(crate) async fn place_schedules(&self, at: Located) -> Result<(), ApiError> {
        let placed = SchedulePlaced {
            org: at.org.as_uuid(),
            project: at.project.as_uuid(),
        };
        let absent = |held: &Placed| (!held.held.contains_key(&at)).then(|| held.next_seq());
        self.list_once(&placed, absent).await
    }

    /// For the scheduler: unplaces the project `at` where its ledger, just read, has held no
    /// schedule for [`IDLE`], `emptied` being when its last went (none: it held none since it
    /// was placed), then reads the ledger again whole and places the project back where a
    /// schedule came meanwhile.
    pub(crate) async fn unplace_if_idle(
        &self,
        at: Located,
        emptied: Option<At>,
    ) -> Result<(), ApiError> {
        let Some(placed) = self.held_placed(|held| held.held.get(&at).copied()) else {
            return Ok(());
        };
        let since = emptied.map_or(placed, |emptied| emptied.max(placed));
        let now = self.inner.harness.clock().now();
        if since
            .timestamp()
            .checked_add(IDLE)
            .is_ok_and(|idle| idle > now)
        {
            return Ok(());
        }
        let gone = ScheduleUnplaced {
            org: at.org.as_uuid(),
            project: at.project.as_uuid(),
        };
        let listed = |held: &Placed| held.held.contains_key(&at).then(|| held.next_seq());
        self.list_once(&gone, listed).await?;
        if self.ledger_apart(at).await?.schedules.is_empty() {
            return Ok(());
        }
        self.place_schedules(at).await
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
    fn placed_scope(&self) -> Scope {
        Scope::of_organization(self.inner.own)
    }

    fn held_placed<R>(&self, look: impl FnOnce(&mut Placed) -> R) -> R {
        let mut held = (self.inner.placed)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        look(&mut held)
    }
}
