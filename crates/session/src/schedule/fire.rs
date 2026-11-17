//! Firing schedules. The server that runs tasks calls [`Server::tick`] (or
//! [`Server::run_scheduler`], which calls it when the next run is due): each due schedule, in
//! every project whose tasks it runs, everyone's, adds one turn to its chat, run as a task with
//! its budget, as whoever made it, that waits for them where it needs them, as a chat does.
//! One server on the store fires them, the holder of its scheduler lease; each run is claimed
//! by its own `task.started`, so even two servers that both believe they hold it start it once
//! (`tasks/run.rs`). Organization by organization, each tick starting one further on, a few
//! projects' ledgers are read at once and each one's due runs started right after, for at most
//! a share of the tick each organization, the next tick going on where it stopped. A run due
//! while the one before is still going in the chat, or while someone's turn is, is skipped and
//! recorded so (`schedule.skipped`); a run refused for any other reason is recorded with its
//! reason. A skip for the reason the latest skip had is recorded once, then only held back. A
//! schedule that missed runs while no server ran it runs once, as soon as one does, never once
//! per missed time (systemd's `Persistent=true`).

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use glave_event::{
    At, Fired, PrincipalId, ScheduleCreated, ScheduleSkipped, Skipped, ThreadId, When,
};
use glave_ports::{BlobStore, Caller, EventLog, Leased, Located, ModelProvider, OrgId};
use jiff::{SignedDuration, Timestamp};
use tokio::task::JoinSet;

use super::info::{due, next_fire};
use crate::choice::Choice;
use crate::cron::{Cron, SHORTEST_GAP};
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::tasks::{Membership, Unstarted, Work, whose};
use crate::words::Sentence;

/// The longest the scheduler sleeps before looking again: schedules made by another process
/// are found within this.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// The shortest time from one tick's start to the next's, so a run due now that a tick did not
/// start is not looked at again at once.
const SOONEST: SignedDuration = SignedDuration::from_secs(1);
/// How long the store's scheduler lease lasts unless renewed: past the longest wait between
/// ticks, so the server firing schedules keeps it from tick to tick, and another takes over
/// this long after one stops without letting go.
const SCHEDULING: Duration = Duration::from_secs(180);
/// Projects' ledgers read at once.
const READS: usize = 16;
/// The longest one organization's projects take of a tick before the next organization's
/// turn: the rest wait for the next tick, which goes on from there.
const SHARE: SignedDuration = SignedDuration::from_secs(10);

/// What came of a run that was due.
enum Ran {
    /// It started, in this chat.
    Started(ThreadId),
    /// It did not start, which is recorded or held back.
    NotStarted,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Starts a run of every schedule due by the clock's now, in every project whose tasks
    /// this server runs, at most one each, and answers with their chats, first letting go of
    /// threads idle too long. A run its person's task limit holds back starts on a later tick.
    pub async fn tick(&self) -> Result<Vec<ThreadId>, ApiError> {
        // Threads idle for 30 minutes are let go even when nothing else happens.
        self.let_go_stale().await;
        // One server on the store fires schedules: the holder of its lease.
        if !self.fires().await? {
            return Ok(Vec::new());
        }
        let projects = self.task_projects().await?;
        let organizations: BTreeSet<OrgId> = projects.iter().map(|at| at.org).collect();
        let mut started = Vec::new();
        for org in in_turn(organizations, self.next_turn()) {
            let theirs = projects.iter().filter(|at| at.org == org).copied();
            started.extend(self.fire_in(org, theirs.collect()).await);
        }
        Ok(started)
    }

    /// Starts the runs due in `projects`, all `org`'s: from where the tick before stopped, a
    /// few ledgers read at once and each one's runs started right after, for at most
    /// [`SHARE`] of the tick; answers their chats.
    async fn fire_in(&self, org: OrgId, mut projects: Vec<Located>) -> Vec<ThreadId> {
        let from = self.resume_at(org, projects.len());
        projects.rotate_left(from);
        let clock = self.inner.harness.clock();
        let until = clock.now().checked_add(SHARE).ok();
        let (mut started, mut done) = (Vec::new(), 0usize);
        for chunk in projects.chunks(READS) {
            if until.is_some_and(|until| clock.now() >= until) {
                let left = projects.len().saturating_sub(done);
                tracing::warn!(%org, left, "the organization's schedules took their share of the tick");
                break;
            }
            let mut reads = JoinSet::new();
            for at in chunk.iter().copied() {
                let server = self.clone();
                reads.spawn(async move { (at, server.catch_up(at).await) });
            }
            let read: Vec<(Located, Result<(), ApiError>)> = reads.join_all().await;
            // Each project's runs right after its ledger was read, in the chunk's order.
            for at in chunk {
                match read
                    .iter()
                    .find(|(read, _)| read == at)
                    .map(|(_, result)| result)
                {
                    Some(Ok(())) => started.extend(self.recover_and_fire(*at).await),
                    // Read again on the next tick.
                    Some(Err(error)) => {
                        let project = at.project;
                        tracing::warn!(%error, %project, "could not read the project's schedules");
                    }
                    None => {}
                }
            }
            done = done.saturating_add(chunk.len());
        }
        self.stopped_at(org, from.saturating_add(done));
        started
    }

    /// Recovers the runs `project`'s ledger, just read, has open that a stopped server left,
    /// then starts its runs due.
    async fn recover_and_fire(&self, project: Located) -> Vec<ThreadId> {
        if let Err(error) = self.recover_in(project).await {
            let project = project.project;
            tracing::warn!(%error, %project, "could not recover the project's runs left open");
        }
        self.fire_due(project).await
    }

    /// Starts every run due in `project`, found in its ledger as just read rather than in what
    /// this server keeps of ledgers, which forgets some past the most it keeps; answers their
    /// chats.
    async fn fire_due(&self, project: Located) -> Vec<ThreadId> {
        let now = self.inner.harness.clock().now();
        let mut due: Vec<(ScheduleCreated, Timestamp)> = (self.ledgers().get(&project))
            .map(|ledger| {
                (ledger.schedules.values())
                    .filter_map(|scheduled| {
                        due(scheduled, now).map(|at| (scheduled.created.clone(), at))
                    })
                    .collect()
            })
            .unwrap_or_default();
        due.sort_by_key(|(schedule, at)| (*at, schedule.schedule));
        let mut started = Vec::new();
        for (schedule, at) in due {
            // Held back by its maker's task limit before it is claimed, so it starts later.
            if let Ok(maker) = whose(schedule.by.as_ref())
                && self.at_task_limit(maker)
            {
                continue;
            }
            if let Ran::Started(chat) = self.fire(project, &schedule, at).await {
                started.push(chat);
            }
        }
        started
    }

    /// Whether this server runs as many tasks of `person`'s as policy allows.
    fn at_task_limit(&self, person: Option<PrincipalId>) -> bool {
        self.running_for(person) >= self.most_running()
    }

    /// Whether this server fires the store's schedules: it takes their lease, one for the
    /// store, or renews it, for [`SCHEDULING`]; none while another server holds it.
    async fn fires(&self) -> Result<bool, ApiError> {
        let (holder, own) = (self.holder()?, self.inner.own);
        let taken = self
            .inner
            .leases
            .acquire(own, Leased::Scheduler, holder, SCHEDULING)
            .await
            .map_err(|error| ApiError::internal(&error))?;
        let mut scheduling = self.scheduling();
        if let Some(lease) = taken {
            scheduling.insert(own, lease);
            Ok(true)
        } else {
            scheduling.remove(&own);
            Ok(false)
        }
    }

    /// Holds back `schedule`'s run due at `at` until the schedule's next time, in memory only.
    fn hold_back(&self, project: Located, schedule: &ScheduleCreated, at: Timestamp) {
        if let Some(scheduled) = self
            .ledgers()
            .get_mut(&project)
            .and_then(|ledger| ledger.schedules.get_mut(&schedule.schedule))
        {
            scheduled.held = scheduled.held.max(Some(At::from(at)));
        }
    }

    /// Runs the schedules until `stop` finishes: a tick whenever the next run is due or a task
    /// ends, at least every minute, for schedules other processes made, and never sooner than
    /// a second after the tick before began.
    pub async fn run_scheduler(&self, stop: impl Future<Output = ()>) {
        let mut stop = std::pin::pin!(stop);
        loop {
            let began = self.inner.harness.clock().now();
            // A failing tick is tried again on the next one.
            let _ = self.tick().await;
            let now = self.inner.harness.clock().now();
            let wait = self.next_due().map_or(LOOK_AGAIN, |next| {
                Duration::try_from(next.duration_since(now)).unwrap_or(Duration::ZERO)
            });
            let soonest = began
                .checked_add(SOONEST)
                .map_or(Duration::ZERO, |soonest| {
                    Duration::try_from(soonest.duration_since(now)).unwrap_or(Duration::ZERO)
                });
            tokio::select! {
                () = &mut stop => return,
                () = tokio::time::sleep(wait.min(LOOK_AGAIN).max(soonest)) => {}
                // A run the task limit held back may start now, though not before a second
                // since this tick began.
                () = async {
                    self.inner.tasks_ended.notified().await;
                    tokio::time::sleep(soonest).await;
                } => {}
            }
        }
    }

    /// When the next tick would start a run, of the schedules this server fires: never sooner
    /// than a second from now, so a run due now that a tick would not start is not looked at
    /// again at once. One its maker's task limit holds back is left to the tick that follows a
    /// task ending, or the one at least every minute.
    pub fn next_due(&self) -> Option<Timestamp> {
        if !self.scheduling().contains_key(&self.inner.own) {
            return None;
        }
        let now = self.inner.harness.clock().now();
        let times: Vec<(Option<PrincipalId>, Timestamp)> = (self.ledgers().values())
            .flat_map(|ledger| ledger.schedules.values())
            .filter_map(|scheduled| {
                // One glave can't tell who made is refused at its time, whatever the limit.
                let maker = whose(scheduled.created.by.as_ref()).ok().flatten();
                next_fire(scheduled).map(|next| (maker, next))
            })
            .collect();
        let next = (times.into_iter())
            .filter(|(maker, next)| *next > now || !self.at_task_limit(*maker))
            .map(|(_, next)| next)
            .min()?;
        Some(next.max(now.checked_add(SOONEST).ok()?))
    }

    /// Starts `schedule`'s run that was due at `at`: a new turn in its chat in `project`, as
    /// whoever made it, once its claim lands in the ledger; unless the run before is still
    /// going there, on this server or another, or someone's turn is, or glave can't tell who
    /// made it, or they are no longer in its organization, or it runs more often than a
    /// schedule may. While its maker's place in the organization can't be told, it is held
    /// back.
    async fn fire(&self, project: Located, schedule: &ScheduleCreated, at: Timestamp) -> Ran {
        let Some(chat) = schedule.thread else {
            return Ran::NotStarted;
        };
        let fired = Fired {
            schedule: schedule.schedule,
            due: At::from(at),
        };
        // A run still going, on this server or another, is open in the ledger.
        let going =
            (self.ledgers().get(&project)).is_some_and(|ledger| ledger.open.contains_key(&chat));
        let maker = if going {
            Err(crate::held::busy(chat))
        } else {
            self.run_as(project, schedule).await
        };
        let caller = match maker {
            Ok(Some(caller)) => caller,
            Ok(None) => {
                self.hold_back(project, schedule, at);
                return Ran::NotStarted;
            }
            Err(refused) => {
                return self
                    .not_started(project, schedule, at, &refused, false)
                    .await;
            }
        };
        let work = Work(schedule.prompt.clone(), Choice::default());
        match (self.start_run(&caller, chat, work, schedule.budget, fired)).await {
            Ok(Some(_)) => Ran::Started(chat),
            // Another server's claim landed first.
            Ok(None) => Ran::NotStarted,
            Err(Unstarted { refused, claimed }) => {
                self.not_started(project, schedule, at, &refused, claimed)
                    .await
            }
        }
    }

    /// Who `schedule`'s run in `project` acts as: whoever made it, in its project's
    /// organization, as glave acts for them on its own. Refused when glave can't tell who that
    /// is, when they were removed from it, or when the schedule runs more often than one may
    /// (made before that was refused); none, to hold the run back, while the organization's
    /// people can't be read to tell them there, since being removed is what ends their runs.
    async fn run_as(
        &self,
        project: Located,
        schedule: &ScheduleCreated,
    ) -> Result<Option<Caller>, ApiError> {
        if let When::Cron(text) = &schedule.when {
            Cron::for_a_schedule(text).map_err(|why| ApiError::new(code::INVALID_PARAMS, why))?;
        }
        let caller = self.acting_as(whose(schedule.by.as_ref())?, project.org);
        match self.membership(&caller).await {
            Ok(Membership::Member) => Ok(Some(caller)),
            Ok(Membership::Removed) => {
                Err(ApiError::new(code::NOT_ALLOWED, Sentence::MakerRemoved))
            }
            Ok(Membership::Unknown) => Ok(None),
            Err(error) => {
                let org = project.org;
                tracing::warn!(%error, %org, "could not read the organization's people");
                Ok(None)
            }
        }
    }

    /// Records `schedule`'s run due at `at` skipped for `refused`, unclaimed once for the same
    /// reason as the latest skip; held back when that can't be recorded. One this server
    /// `claimed` is recorded whatever its claim says of it; any other only while the ledger
    /// shows nothing of it, so servers on one store record it once.
    async fn not_started(
        &self,
        project: Located,
        schedule: &ScheduleCreated,
        at: Timestamp,
        refused: &ApiError,
        claimed: bool,
    ) -> Ran {
        let why = skipped_for(refused);
        // Once for the same reason as the latest skip, then only held back; but a run this
        // server claimed is settled in the ledger whatever its reason.
        if !claimed && self.skipped_for_the_same(project, schedule, &why) {
            self.hold_back(project, schedule, at);
            return Ran::NotStarted;
        }
        let skipped = ScheduleSkipped {
            schedule: schedule.schedule,
            due: At::from(at),
            why,
        };
        let recorded = if claimed {
            self.record_in_ledger(project, &skipped).await
        } else {
            let there =
                |ledger: &crate::ledger::Ledger| ledger.settled(skipped.schedule, skipped.due);
            self.record_unless(project, &skipped, there)
                .await
                .map(|_| ())
        };
        if let Err(error) = recorded {
            tracing::warn!(%error, schedule = %schedule.schedule, "a skipped run was not recorded");
            self.hold_back(project, schedule, at);
        }
        Ran::NotStarted
    }

    /// Whether `schedule`'s latest run was skipped, for `why`.
    fn skipped_for_the_same(
        &self,
        project: Located,
        schedule: &ScheduleCreated,
        why: &Skipped,
    ) -> bool {
        (self.ledgers().get(&project))
            .and_then(|ledger| ledger.schedules.get(&schedule.schedule))
            .is_some_and(|scheduled| scheduled.skipped.as_ref() == Some(why))
    }
}

/// Why a run `refused` was skipped: as the one before still goes when a turn runs in its chat,
/// here or on another server; else for the refusal's reason, which each reader reads in their
/// language.
fn skipped_for(refused: &ApiError) -> Skipped {
    let too_often = Sentence::CronTooOften {
        minutes: SHORTEST_GAP,
    };
    let reason = match refused.code {
        code::TURN_RUNNING | code::RUNNING_ELSEWHERE => return Skipped::Going,
        // Finer than its code: whoever made it is gone, or can't be told, or it runs too often.
        _ if refused.message.is(&Sentence::MakerRemoved) => "MAKER_REMOVED",
        _ if refused.message.is(&Sentence::MakerUnknown) => "MAKER_UNKNOWN",
        _ if refused.message.is(&too_often) => "TOO_OFTEN",
        other => code::name_of(other).unwrap_or_default(),
    };
    Skipped::Refused {
        reason: reason.to_owned(),
    }
}

/// `organizations` in the order a tick takes them: from the `start`th on, round to the first,
/// so each tick starts one further on.
fn in_turn(organizations: BTreeSet<OrgId>, start: usize) -> Vec<OrgId> {
    let mut order: Vec<OrgId> = organizations.into_iter().collect();
    if let Some(by) = start.checked_rem(order.len()) {
        order.rotate_left(by);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tick_starts_one_organization_further_on() {
        let org = |n| OrgId::from_uuid(uuid::Uuid::from_u128(n));
        let all: BTreeSet<OrgId> = [org(1), org(2), org(3)].into();
        assert_eq!(in_turn(all.clone(), 0), vec![org(1), org(2), org(3)]);
        assert_eq!(in_turn(all.clone(), 1), vec![org(2), org(3), org(1)]);
        assert_eq!(in_turn(all.clone(), 5), vec![org(3), org(1), org(2)]);
        assert_eq!(in_turn(BTreeSet::new(), 7), Vec::new());
    }
}
