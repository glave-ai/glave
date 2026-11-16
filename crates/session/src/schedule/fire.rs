//! Firing schedules. The server that runs tasks calls [`Server::tick`] (or
//! [`Server::run_scheduler`], which calls it when the next run is due): each due schedule, in
//! every project whose tasks it runs, adds one turn to its chat, run as a task with its budget
//! that waits for the person where it needs them, as a chat does. A run due while the one before
//! is still going in the chat, or while someone's turn is, is skipped and recorded so
//! (`schedule.skipped`). A schedule that missed runs while no server ran it runs once, as soon
//! as one does, never once per missed time (systemd's `Persistent=true`); a run that cannot
//! start is held back until the schedule's next time.

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use glave_event::{At, Fired, ScheduleCreated, ScheduleSkipped, ThreadId};
use glave_ports::{BlobStore, Caller, EventLog, Leased, Located, ModelProvider, OrgId};
use jiff::Timestamp;

use super::{due, next_run};
use crate::choice::Choice;
use crate::ledger::own_scope;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::tasks::Work;

/// The longest the scheduler sleeps before looking again: schedules made by another process
/// are found within this.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// How long the organization's scheduler lease lasts unless renewed: past the longest wait
/// between ticks, so the server firing its schedules keeps it from tick to tick, and another
/// takes over this long after one stops without letting go.
const SCHEDULING: Duration = Duration::from_secs(180);

/// What came of a run that was due.
enum Ran {
    /// It started, in this thread.
    Started(ThreadId),
    /// The run before was still going in its chat, or someone's turn was: skipped.
    Skipped,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Starts a run of every schedule due by the clock's now, in every project whose tasks
    /// this server runs, at most one each, and answers with their threads, first letting go
    /// of threads idle too long. A run the task limit holds back starts on a later tick; one
    /// that cannot start is held back until the schedule's next time.
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
                // Only the person on this computer's run, as a run is theirs; one made by
                // someone signed in from elsewhere waits for #410, leaving no thread behind.
                let theirs = ledger
                    .schedules
                    .values()
                    .filter(|it| it.created.by.is_none());
                theirs.filter_map(move |scheduled| {
                    due(scheduled, now).map(|at| (*project, scheduled.created.clone(), at))
                })
            })
            .collect();
        due.sort_by_key(|(_, schedule, at)| (*at, schedule.schedule));
        let mut started = Vec::new();
        for (project, schedule, at) in due {
            if self
                .inner
                .limits
                .max_running
                .is_some_and(|max| self.tasks().len() >= max)
            {
                break;
            }
            match self.fire(project, &schedule, at).await {
                Ok(Ran::Started(thread)) => started.push(thread),
                Ok(Ran::Skipped) => {}
                Err(error) => {
                    tracing::warn!(%error, schedule = %schedule.schedule, "a run could not start");
                    self.hold_back(project, &schedule, at);
                }
            }
        }
        Ok(started)
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

    /// Holds back `schedule`'s run that was due at `at` and could not start, until the
    /// schedule's next time.
    fn hold_back(&self, project: Located, schedule: &ScheduleCreated, at: Timestamp) {
        if let Some(scheduled) = self
            .ledgers()
            .get_mut(&project)
            .and_then(|ledger| ledger.schedules.get_mut(&schedule.schedule))
        {
            scheduled.held = scheduled.held.max(Some(At::from(at)));
        }
    }

    /// Runs the schedules until `stop` finishes: a tick whenever the next run is due, and at
    /// least every minute, for schedules other processes made.
    pub async fn run_scheduler(&self, stop: impl Future<Output = ()>) {
        let mut stop = std::pin::pin!(stop);
        loop {
            // A failing tick is tried again on the next one.
            let _ = self.tick().await;
            let now = self.inner.harness.clock().now();
            let wait = self
                .next_due()
                .and_then(|next| {
                    let millis = next.as_millisecond().saturating_sub(now.as_millisecond());
                    u64::try_from(millis).ok()
                })
                .map_or(LOOK_AGAIN, |millis| {
                    Duration::from_millis(millis).min(LOOK_AGAIN)
                });
            tokio::select! {
                () = &mut stop => return,
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// When the next schedule is due, if any is, of those this server has read.
    pub fn next_due(&self) -> Option<Timestamp> {
        let now = self.inner.harness.clock().now();
        self.ledgers()
            .values()
            .flat_map(|ledger| ledger.schedules.values())
            .filter_map(|scheduled| next_run(scheduled, now))
            .min()
    }

    /// Starts `schedule`'s run that was due at `at`: a new turn in its chat in `project`, unless
    /// the run before is still going there, here or on another server, or someone's turn is.
    async fn fire(
        &self,
        project: Located,
        schedule: &ScheduleCreated,
        at: Timestamp,
    ) -> Result<Ran, ApiError> {
        // As the person on this computer, whose run it is, in its project's organization.
        let caller = Caller {
            org: Some(project.org),
            ..Caller::default()
        };
        let work = Work(schedule.prompt.clone(), Choice::default());
        let fired = Fired {
            schedule: schedule.schedule,
            due: At::from(at),
        };
        let budget = Some(schedule.budget);
        let Some(chat) = schedule.thread else {
            // Made before schedules had chats: each run is a thread of its own, as then.
            let thread = self
                .new_thread(&caller, own_scope(project), None, true, &Choice::default())
                .await?;
            self.start_task(&caller, thread, work, budget, Some(fired))
                .await?;
            return Ok(Ran::Started(thread));
        };
        // A run still going, on this server or another, is open in the ledger.
        let going =
            (self.ledgers().get(&project)).is_some_and(|ledger| ledger.open.contains_key(&chat));
        let started = if going {
            Err(crate::held::busy(chat))
        } else {
            self.start_task(&caller, chat, work, budget, Some(fired))
                .await
        };
        match started {
            Ok(_) => Ok(Ran::Started(chat)),
            Err(refused) if refused.code == code::TURN_RUNNING => {
                let skipped = ScheduleSkipped {
                    schedule: schedule.schedule,
                    due: fired.due,
                };
                self.record_in_ledger(project, &skipped).await?;
                Ok(Ran::Skipped)
            }
            Err(error) => Err(error),
        }
    }
}
