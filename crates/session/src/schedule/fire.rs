//! Firing schedules. The server that runs tasks calls [`Server::tick`] (or
//! [`Server::run_scheduler`], which calls it when the next run is due): each due schedule, in
//! every project whose tasks it runs, everyone's, adds one turn to its chat, run as a task with
//! its budget, as whoever made it, that waits for them where it needs them, as a chat does. A
//! run due while the one
//! before is still going in the chat, or while someone's turn is, is skipped and recorded so
//! once (`schedule.skipped`); a run refused for any other reason is recorded with why. A
//! schedule that missed runs while no server ran it runs once, as soon as one does, never once
//! per missed time (systemd's `Persistent=true`).

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
use crate::tasks::{Work, whose};
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
        // One server fires each organization's schedules: the holder of its lease.
        let projects = self.task_projects().await?;
        let mut firing = BTreeSet::new();
        for org in projects.iter().map(|at| at.org).collect::<BTreeSet<_>>() {
            if self.fires(org).await? {
                firing.insert(org);
            }
        }
        for project in projects.iter().filter(|at| firing.contains(&at.org)) {
            // A ledger that cannot be read now is read again on the next tick.
            if let Err(error) = self.catch_up(*project).await {
                let project = project.project;
                tracing::warn!(%error, %project, "could not read the project's schedules");
            }
        }
        let now = self.inner.harness.clock().now();
        let mut due: Vec<(Located, ScheduleCreated, Timestamp)> = self
            .ledgers()
            .iter()
            .filter(|(at, _)| firing.contains(&at.org))
            .flat_map(|(project, ledger)| {
                (ledger.schedules.values()).filter_map(move |scheduled| {
                    due(scheduled, now).map(|at| (*project, scheduled.created.clone(), at))
                })
            })
            .collect();
        due.sort_by_key(|(_, schedule, at)| (*at, schedule.schedule));
        let mut started = Vec::new();
        for (project, schedule, at) in due {
            if self.at_task_limit(whose(schedule.by.as_ref())) {
                continue;
            }
            if let Ran::Started(chat) = self.fire(project, &schedule, at).await {
                started.push(chat);
            }
        }
        Ok(started)
    }

    /// Whether this server runs as many tasks of `person`'s as policy allows.
    fn at_task_limit(&self, person: Option<PrincipalId>) -> bool {
        (self.inner.limits.max_running).is_some_and(|max| self.running_for(person) >= max)
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
                let maker = whose(scheduled.created.by.as_ref());
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
    /// whoever made it, unless the run before is still going there, on this server or another,
    /// or someone's turn is, or they are no longer in its organization.
    async fn fire(&self, project: Located, schedule: &ScheduleCreated, at: Timestamp) -> Ran {
        let Some(chat) = schedule.thread else {
            return Ran::NotStarted;
        };
        // As whoever made it, with what they may do, spend and work on, in its project's
        // organization; a sign-in of theirs ending changes nothing, being removed does.
        let caller = Caller {
            person: whose(schedule.by.as_ref()),
            org: Some(project.org),
            ..Caller::default()
        };
        let fired = Fired {
            schedule: schedule.schedule,
            due: At::from(at),
        };
        // A run still going, on this server or another, is open in the ledger.
        let going =
            (self.ledgers().get(&project)).is_some_and(|ledger| ledger.open.contains_key(&chat));
        let started = if going {
            Err(crate::held::busy(chat))
        } else if self.maker_removed(&caller).await {
            Err(ApiError::new(code::NOT_ALLOWED, Sentence::MakerRemoved))
        } else {
            let work = Work(schedule.prompt.clone(), Choice::default());
            let budget = Some(schedule.budget);
            self.start_task(&caller, chat, work, budget, Some(fired))
                .await
        };
        let why = match started {
            Ok(_) => return Ran::Started(chat),
            Err(refused) if refused.code == code::TURN_RUNNING => Skipped::Going,
            Err(refused) => Skipped::Refused {
                why: refused.message.to_string(),
            },
        };
        // Once while the same run still goes: then only held back.
        let again = why == Skipped::Going && self.skipped_going(project, schedule);
        let skipped = ScheduleSkipped {
            schedule: schedule.schedule,
            due: fired.due,
            why,
        };
        if again {
            self.hold_back(project, schedule, at);
        } else if let Err(error) = self.record_in_ledger(project, &skipped).await {
            tracing::warn!(%error, schedule = %schedule.schedule, "a skipped run was not recorded");
            self.hold_back(project, schedule, at);
        }
        Ran::NotStarted
    }

    /// Whether `caller`, a schedule's maker, was removed from its organization: never the
    /// server's own person. One whose organization's people cannot be read now is taken as
    /// still there; what they run is ended once a read finds them removed.
    async fn maker_removed(&self, caller: &Caller) -> bool {
        let (Some(person), Some(org)) = (caller.person, caller.org) else {
            return false;
        };
        if let Err(error) = self.catch_up_people(org).await {
            tracing::warn!(%error, %org, "could not read the organization's people");
        }
        self.people(org, |people| people.removed(person))
    }

    /// Whether `schedule`'s latest run was skipped as the one before still went.
    fn skipped_going(&self, project: Located, schedule: &ScheduleCreated) -> bool {
        (self.ledgers().get(&project))
            .and_then(|ledger| ledger.schedules.get(&schedule.schedule))
            .is_some_and(|scheduled| scheduled.skipped == Some(Skipped::Going))
    }
}
