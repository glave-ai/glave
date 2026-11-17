//! Firing schedules. The server that runs tasks calls [`Server::tick`] (or
//! [`Server::run_scheduler`], which calls it when the next run is due): each due schedule, in
//! every project whose tasks it runs, adds one turn to its chat, run as a task with its budget
//! that waits for the person where it needs them, as a chat does. A run due while the one
//! before is still going in the chat, or while someone's turn is, is skipped and recorded so
//! (`schedule.skipped`); a run refused for any other reason is recorded with its reason. A skip
//! for the reason the latest skip had is recorded once, then only held back. A schedule that
//! missed runs while no server ran it runs once, as soon as one does, never once per missed
//! time (systemd's `Persistent=true`).

use std::future::Future;
use std::time::Duration;

use glave_event::{At, Fired, ScheduleCreated, ScheduleSkipped, Skipped, ThreadId};
use glave_ports::{BlobStore, Caller, EventLog, Leased, Located, ModelProvider};
use jiff::{SignedDuration, Timestamp};

use super::info::{due, next_fire};
use crate::choice::Choice;
use crate::ledger::Scheduled;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::tasks::{Unstarted, Work};

/// The longest the scheduler sleeps before looking again: schedules made by another process
/// are found within this.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// The shortest it sleeps, so a run due now that a tick did not start is not looked at again
/// at once.
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
    /// threads idle too long. A run the task limit holds back starts on a later tick.
    pub async fn tick(&self) -> Result<Vec<ThreadId>, ApiError> {
        // Threads idle for 30 minutes are let go even when nothing else happens.
        self.let_go_stale().await;
        // One server on the store fires schedules: the holder of its lease.
        if !self.fires().await? {
            return Ok(Vec::new());
        }
        let projects = self.task_projects().await?;
        for project in &projects {
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
            .flat_map(|(project, ledger)| {
                (ledger.schedules.values())
                    .filter(|it| fires_here(it))
                    .filter_map(move |scheduled| {
                        due(scheduled, now).map(|at| (*project, scheduled.created.clone(), at))
                    })
            })
            .collect();
        due.sort_by_key(|(_, schedule, at)| (*at, schedule.schedule));
        let mut started = Vec::new();
        for (project, schedule, at) in due {
            if self.at_task_limit() {
                break;
            }
            if let Ran::Started(chat) = self.fire(project, &schedule, at).await {
                started.push(chat);
            }
        }
        Ok(started)
    }

    /// Whether this server runs as many tasks as policy allows: a run waiting for the person
    /// runs nothing meanwhile, and counts toward no limit.
    fn at_task_limit(&self) -> bool {
        self.working(|_| true) >= self.most_running()
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
    /// ends, at least every minute, for schedules other processes made, and at most every
    /// second.
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
                // A run the task limit held back may start now.
                () = self.inner.tasks_ended.notified() => {}
            }
        }
    }

    /// When the next tick would start a run, of the schedules this server fires: never sooner
    /// than a second from now. A run due now that the task limit holds back is left to the
    /// tick that follows a task ending, or the one at least every minute.
    pub fn next_due(&self) -> Option<Timestamp> {
        if !self.scheduling().contains_key(&self.inner.own) {
            return None;
        }
        let now = self.inner.harness.clock().now();
        let held_back = self.at_task_limit();
        let next = (self.ledgers().values())
            .flat_map(|ledger| ledger.schedules.values())
            .filter(|scheduled| fires_here(scheduled))
            .filter_map(next_fire)
            .filter(|next| *next > now || !held_back)
            .min()?;
        let soonest = now
            .checked_add(SignedDuration::try_from(SOONEST).ok()?)
            .ok()?;
        Some(next.max(soonest))
    }

    /// Starts `schedule`'s run that was due at `at`: a new turn in its chat in `project`, once
    /// its claim lands in the ledger (`tasks/run.rs`), unless the run before is still going
    /// there, on this server or another, or someone's turn is.
    async fn fire(&self, project: Located, schedule: &ScheduleCreated, at: Timestamp) -> Ran {
        let Some(chat) = schedule.thread else {
            return Ran::NotStarted;
        };
        // As the person on this computer, whose run it is, in its project's organization.
        let caller = Caller {
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
            Err(Unstarted {
                refused: crate::held::busy(chat),
                claimed: false,
            })
        } else {
            let work = Work(schedule.prompt.clone(), Choice::default());
            self.start_run(&caller, chat, work, schedule.budget, fired)
                .await
        };
        let (why, claimed) = match started {
            Ok(Some(_)) => return Ran::Started(chat),
            // Another server's claim landed first.
            Ok(None) => return Ran::NotStarted,
            Err(Unstarted { refused, claimed }) => (skipped_for(&refused), claimed),
        };
        // Once for the same reason as the latest skip, then only held back; but a run this
        // server claimed is settled in the ledger whatever its reason.
        if !claimed && self.skipped_for_the_same(project, schedule, &why) {
            self.hold_back(project, schedule, at);
            return Ran::NotStarted;
        }
        let skipped = ScheduleSkipped {
            schedule: schedule.schedule,
            due: fired.due,
            why,
        };
        // A run not claimed is recorded only while the ledger shows nothing of it, so servers
        // on one store record it once.
        let there = |ledger: &crate::ledger::Ledger| ledger.settled(skipped.schedule, skipped.due);
        let recorded = if claimed {
            self.record_in_ledger(project, &skipped).await
        } else {
            (self.record_unless(project, &skipped, there).await).map(|_| ())
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
    match refused.code {
        code::TURN_RUNNING | code::RUNNING_ELSEWHERE => Skipped::Going,
        other => Skipped::Refused {
            reason: code::name_of(other).unwrap_or_default().to_owned(),
        },
    }
}

/// Whether `scheduled`'s runs start on this server: those of the person on this computer, as
/// a run is theirs; one made by someone signed in from elsewhere waits for #410.
fn fires_here(scheduled: &Scheduled) -> bool {
    scheduled.created.by.is_none()
}
