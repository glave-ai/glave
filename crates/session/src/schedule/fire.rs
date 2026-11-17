//! Firing schedules. The server that runs tasks calls [`Server::tick`] (or
//! [`Server::run_scheduler`], which calls it when the next run is due): each due schedule, in
//! every project whose tasks it runs, everyone's, adds one turn to its chat, run as a task with
//! its budget, as whoever made it, that waits for them where it needs them, as a chat does.
//! Each project's runs are found and started right after its ledger is read, organization by
//! organization, each tick starting one organization further on, so none waits behind the
//! others every time. A run is claimed in the ledger (`schedule.fired`) before its turn starts,
//! and only the server whose claim lands starts it. A run due while the one before is still
//! going in the chat, or while someone's turn is, is skipped and recorded so once
//! (`schedule.skipped`); a run refused for any other reason is recorded with why. A schedule
//! that missed runs while no server ran it runs once, as soon as one does, never once per
//! missed time (systemd's `Persistent=true`).

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use glave_event::{At, Fired, PrincipalId, ScheduleCreated, ScheduleSkipped, Skipped, ThreadId};
use glave_ports::{BlobStore, Caller, EventLog, Leased, Located, ModelProvider, OrgId};
use jiff::{SignedDuration, Timestamp};

use super::info::{due, next_fire};
use crate::choice::Choice;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::tasks::{Work, acting_for, whose};
use crate::words::Sentence;

/// The longest the scheduler sleeps before looking again: schedules made by another process
/// are found within this.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// The shortest it sleeps: a run due now that the next tick would not start, held back by the
/// task limit, waits at least this long rather than being looked at again at once.
const SOONEST: Duration = Duration::from_secs(1);
/// How long the organization's scheduler lease lasts unless renewed: past the longest wait
/// between ticks, so the server firing its schedules keeps it from tick to tick, and another
/// takes over this long after one stops without letting go.
const SCHEDULING: Duration = Duration::from_secs(180);

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
        let projects = self.task_projects().await?;
        let organizations: BTreeSet<OrgId> = projects.iter().map(|at| at.org).collect();
        let mut started = Vec::new();
        for org in in_turn(organizations, self.next_turn()) {
            // One server fires each organization's schedules: the holder of its lease.
            match self.fires(org).await {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    tracing::warn!(%error, %org, "could not take the organization's scheduler lease");
                    continue;
                }
            }
            for project in projects.iter().filter(|at| at.org == org) {
                started.extend(self.fire_due(*project).await);
            }
        }
        Ok(started)
    }

    /// Starts every run due in `project`, found in its ledger as just read rather than in what
    /// this server keeps of ledgers, which forgets some past the most it keeps; answers their
    /// chats.
    async fn fire_due(&self, project: Located) -> Vec<ThreadId> {
        // A ledger that cannot be read now is read again on the next tick.
        if let Err(error) = self.catch_up(project).await {
            let project = project.project;
            tracing::warn!(%error, %project, "could not read the project's schedules");
            return Vec::new();
        }
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

    /// Whether this server fires `org`'s schedules: it takes their lease, or renews it, for
    /// [`SCHEDULING`]; none while another server holds it.
    async fn fires(&self, org: OrgId) -> Result<bool, ApiError> {
        let holder = self.holder()?;
        let taken = self
            .inner
            .leases
            .acquire(org, Leased::Scheduler, holder, SCHEDULING)
            .await
            .map_err(|error| ApiError::internal(&error))?;
        let mut scheduling = self.scheduling();
        if let Some(lease) = taken {
            scheduling.insert(org, lease);
            Ok(true)
        } else {
            scheduling.remove(&org);
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

    /// Runs the schedules until `stop` finishes: a tick whenever the next run is due, at least
    /// every minute, for schedules other processes made, and at most every second.
    pub async fn run_scheduler(&self, stop: impl Future<Output = ()>) {
        let mut stop = std::pin::pin!(stop);
        loop {
            // A failing tick is tried again on the next one.
            let _ = self.tick().await;
            let now = self.inner.harness.clock().now();
            let wait = self.next_due().map_or(LOOK_AGAIN, |next| {
                let until = next.duration_since(now);
                Duration::try_from(until).unwrap_or(SOONEST).min(LOOK_AGAIN)
            });
            tokio::select! {
                () = &mut stop => return,
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// When the next tick would start a run, of the schedules this server fires: never sooner
    /// than a second from now, so a run due now that a tick would not start is not looked at
    /// again at once. One its maker's task limit holds back is left to the tick at least every
    /// minute, which finds it once a task of theirs ends.
    pub fn next_due(&self) -> Option<Timestamp> {
        let now = self.inner.harness.clock().now();
        let firing: BTreeSet<OrgId> = self.scheduling().keys().copied().collect();
        let times: Vec<(Option<PrincipalId>, Timestamp)> = self
            .ledgers()
            .iter()
            .filter(|(at, _)| firing.contains(&at.org))
            .flat_map(|(_, ledger)| ledger.schedules.values())
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
        let soonest = now
            .checked_add(SignedDuration::try_from(SOONEST).ok()?)
            .ok()?;
        Some(next.max(soonest))
    }

    /// Starts `schedule`'s run that was due at `at`: a new turn in its chat in `project`, as
    /// whoever made it, once this server's claim to it lands in the ledger; unless the run
    /// before is still going there, on this server or another, or someone's turn is, or glave
    /// can't tell who made it, or they are no longer in its organization. While its
    /// organization's people can't be read, or the claim can't be written, it is held back.
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
            self.maker(project, schedule).await
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
        // Only the server whose claim lands starts the run.
        let there = |ledger: &crate::ledger::Ledger| ledger.settled(fired.schedule, fired.due);
        match self.record_unless(project, &fired, there).await {
            Ok(true) => {}
            Ok(false) => return Ran::NotStarted,
            Err(error) => {
                tracing::warn!(%error, schedule = %schedule.schedule, "a run could not be claimed");
                self.hold_back(project, schedule, at);
                return Ran::NotStarted;
            }
        }
        let work = Work(schedule.prompt.clone(), Choice::default());
        let budget = Some(schedule.budget);
        match (self.start_task(&caller, chat, work, budget, Some(fired))).await {
            Ok(_) => Ran::Started(chat),
            Err(refused) => {
                self.not_started(project, schedule, at, &refused, true)
                    .await
            }
        }
    }

    /// Who `schedule`'s run in `project` acts as: whoever made it, in its project's
    /// organization, as glave acts for them on its own. Refused when glave can't tell who that
    /// is, or they were removed from it; none, to hold the run back, while its people can't be
    /// read, since being removed is what ends their runs.
    async fn maker(
        &self,
        project: Located,
        schedule: &ScheduleCreated,
    ) -> Result<Option<Caller>, ApiError> {
        let caller = acting_for(whose(schedule.by.as_ref())?, project.org);
        match self.removed_from_org(&caller).await {
            Ok(false) => Ok(Some(caller)),
            Ok(true) => Err(ApiError::new(code::NOT_ALLOWED, Sentence::MakerRemoved)),
            Err(error) => {
                let org = project.org;
                tracing::warn!(%error, %org, "could not read the organization's people");
                Ok(None)
            }
        }
    }

    /// Records `schedule`'s run due at `at` skipped for `refused`, once while the same run
    /// still goes; held back when that can't be recorded. One this server `claimed` is recorded
    /// whatever its claim says of it; any other only while the ledger shows nothing of it, so
    /// servers on one store record it once.
    async fn not_started(
        &self,
        project: Located,
        schedule: &ScheduleCreated,
        at: Timestamp,
        refused: &ApiError,
        claimed: bool,
    ) -> Ran {
        let why = match refused.code {
            code::TURN_RUNNING => Skipped::Going,
            _ => Skipped::Refused {
                why: refused.message.to_string(),
            },
        };
        // Once while the same run still goes: then only held back.
        if why == Skipped::Going && self.skipped_going(project, schedule) {
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

    /// Whether `schedule`'s latest run was skipped as the one before still went.
    fn skipped_going(&self, project: Located, schedule: &ScheduleCreated) -> bool {
        (self.ledgers().get(&project))
            .and_then(|ledger| ledger.schedules.get(&schedule.schedule))
            .is_some_and(|scheduled| scheduled.skipped == Some(Skipped::Going))
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
