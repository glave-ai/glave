//! Schedules: a prompt or a goal that runs as a task at set times, once (`at`) or whenever a
//! cron expression matches (in UTC). Schedules live in their project's ledger. The server that
//! runs tasks calls [`Server::tick`] (or [`Server::run_scheduler`], which calls it when the
//! next run is due): each due schedule, in every project whose tasks it runs, starts one task in
//! a new thread, on a copy of its own. A schedule that missed runs while no server ran it runs
//! once, as soon as one does, never once per missed time (systemd's `Persistent=true`); a run
//! that cannot start is skipped until the schedule's next time. Every run has a token budget.

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use glave_event::{At, Fired, ScheduleCreated, ScheduleDeleted, ScheduleId, ThreadId, When};
use glave_harness::Goal;
use glave_ports::{BlobStore, Caller, EventLog, Leased, Located, ModelProvider, OrgId, ProjectId};
use jiff::Timestamp;

use crate::choice::Choice;
use crate::cron::Cron;
use crate::ledger::{Scheduled, own_scope};
use crate::protocol::{ApiError, Reply, ScheduleInfo, code};
use crate::server::Server;
use crate::tasks::Work;
use crate::words::Sentence;

/// The longest the scheduler sleeps before looking again: schedules made by another process
/// are found within this.
const LOOK_AGAIN: Duration = Duration::from_secs(60);
/// How long the organization's scheduler lease lasts unless renewed: past the longest wait
/// between ticks, so the server firing its schedules keeps it from tick to tick, and another
/// takes over this long after one stops without letting go.
const SCHEDULING: Duration = Duration::from_secs(180);
/// Occurrences of a cron expression walked to find the latest one missed.
const MOST_MISSED: usize = 100_000;

/// What `schedule/create` asks for.
pub(crate) struct NewSchedule {
    pub(crate) project: Option<ProjectId>,
    pub(crate) when: When,
    pub(crate) prompt: String,
    pub(crate) goal: bool,
    pub(crate) budget: u64,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    pub(crate) async fn create_schedule(
        &self,
        caller: &Caller,
        new: NewSchedule,
    ) -> Result<Reply, ApiError> {
        if let When::Cron(text) = &new.when {
            Cron::parse(text).map_err(|why| ApiError::new(code::INVALID_PARAMS, why))?;
        }
        if new.budget == 0 {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoBudget,
            ));
        }
        if new.prompt.trim().is_empty() {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoPrompt,
            ));
        }
        let project = self.project_for(caller, new.project).await?.1.located();
        if let Some(max) = self.inner.limits.max_schedules {
            let projects = self.read_ledgers_of(caller).await?;
            let now = self.inner.harness.clock().now();
            let kept = self
                .ledgers()
                .iter()
                .filter(|(id, _)| projects.contains(id))
                .flat_map(|(_, ledger)| ledger.schedules.values())
                .filter(|scheduled| next_run(scheduled, now).is_some())
                .count();
            if kept >= max {
                return Err(ApiError::new(
                    code::LIMIT,
                    Sentence::TooManySchedules { kept, most: max },
                ));
            }
        }
        let now = self.inner.harness.clock().now();
        let schedule = ScheduleId::new(now, self.inner.harness.entropy().bytes()?)?;
        let created = ScheduleCreated {
            schedule,
            when: new.when,
            prompt: new.prompt,
            goal: new.goal,
            budget: new.budget,
            by: caller
                .person
                .filter(|person| Some(*person) != self.inner.person)
                .map(glave_event::Principal::person),
        };
        self.record_in_ledger(project, &created).await?;
        let info = self
            .ledgers()
            .get(&project)
            .and_then(|ledger| ledger.schedules.get(&schedule))
            .map(|scheduled| info(scheduled, self.inner.harness.clock().now()))
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::ScheduleNotKept))?;
        Ok(Reply::Schedule { schedule: info })
    }

    /// The schedules of every project `caller` works in, oldest first.
    pub(crate) async fn list_schedules(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let projects = self.read_ledgers_of(caller).await?;
        let now = self.inner.harness.clock().now();
        let mut schedules: Vec<ScheduleInfo> = self
            .ledgers()
            .iter()
            .filter(|(project, _)| projects.contains(project))
            .flat_map(|(_, ledger)| ledger.schedules.values())
            .map(|scheduled| info(scheduled, now))
            .collect();
        schedules.sort_by_key(|schedule| schedule.schedule);
        Ok(Reply::Schedules { schedules })
    }

    pub(crate) async fn delete_schedule(
        &self,
        caller: &Caller,
        schedule: ScheduleId,
    ) -> Result<Reply, ApiError> {
        let projects = self.read_ledgers_of(caller).await?;
        let project = self
            .ledgers()
            .iter()
            .find(|(project, ledger)| {
                projects.contains(project) && ledger.schedules.contains_key(&schedule)
            })
            .map(|(project, _)| *project);
        let Some(project) = project else {
            return Err(ApiError::new(
                code::NO_SCHEDULE,
                Sentence::NoSchedule {
                    schedule: schedule.to_string(),
                },
            ));
        };
        let deleted = ScheduleDeleted {
            schedule,
            by: caller
                .person
                .filter(|person| Some(*person) != self.inner.person)
                .map(glave_event::Principal::person),
        };
        self.record_in_ledger(project, &deleted).await?;
        Ok(Reply::Done {})
    }

    /// The projects `caller` works in, each one's ledger read up to now.
    async fn read_ledgers_of(&self, caller: &Caller) -> Result<Vec<Located>, ApiError> {
        let projects = self.projects_of(caller).await?;
        for project in &projects {
            self.catch_up(*project).await?;
        }
        Ok(projects)
    }

    /// Starts a task for every schedule due by the clock's now, in every project whose tasks
    /// this server runs, at most one each, and answers with their threads, first letting go
    /// of threads idle too long. A run the task
    /// limit holds back starts on a later tick; one that cannot start is skipped until the
    /// schedule's next time.
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
                Ok(thread) => started.push(thread),
                Err(error) => {
                    tracing::warn!(%error, schedule = %schedule.schedule, "a run could not start");
                    self.skip(project, schedule.schedule, at);
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
    /// schedule's next time: in memory only, so a server started later tries it once more.
    fn skip(&self, project: Located, schedule: ScheduleId, at: Timestamp) {
        if let Some(scheduled) = self
            .ledgers()
            .get_mut(&project)
            .and_then(|ledger| ledger.schedules.get_mut(&schedule))
        {
            scheduled.last = scheduled.last.max(Some(At::from(at)));
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

    /// Starts `schedule`'s run that was due at `at`, in a thread of its own in `project`, which
    /// works on a copy of the project's folder.
    async fn fire(
        &self,
        project: Located,
        schedule: &ScheduleCreated,
        at: Timestamp,
    ) -> Result<ThreadId, ApiError> {
        // As the person on this computer, whose run it is, in its project's organization.
        let caller = Caller {
            org: Some(project.org),
            ..Caller::default()
        };
        let thread = self
            .new_thread(
                &caller,
                // Glave's own: a schedule's run, in its project.
                own_scope(project),
                None,
                true,
                &Choice::default(),
            )
            .await?;
        let work = if schedule.goal {
            Work::Goal(Some(Goal {
                objective: schedule.prompt.clone(),
                token_budget: schedule.budget,
            }))
        } else {
            Work::Turn(schedule.prompt.clone(), Choice::default())
        };
        let fired = Fired {
            schedule: schedule.schedule,
            due: At::from(at),
        };
        self.start_task(&caller, thread, work, Some(schedule.budget), Some(fired))
            .await?;
        Ok(thread)
    }
}

/// The time a run of `scheduled` is due by `now`, if one is: for a cron schedule that missed
/// several, the latest of them.
fn due(scheduled: &Scheduled, now: Timestamp) -> Option<Timestamp> {
    match &scheduled.created.when {
        When::At(at) => (scheduled.last.is_none() && at.timestamp() <= now).then(|| at.timestamp()),
        When::Cron(text) => {
            let cron = Cron::parse(text).ok()?;
            let mut at = cron.next_after(since(scheduled))?;
            if at > now {
                return None;
            }
            for _ in 0..MOST_MISSED {
                match cron.next_after(at) {
                    Some(later) if later <= now => at = later,
                    _ => return Some(at),
                }
            }
            Some(now)
        }
    }
}

/// When `scheduled` runs next after `now`, or is due now; `None` once a one-off has run.
fn next_run(scheduled: &Scheduled, now: Timestamp) -> Option<Timestamp> {
    match &scheduled.created.when {
        When::At(at) => scheduled.last.is_none().then(|| at.timestamp()),
        When::Cron(text) => {
            let cron = Cron::parse(text).ok()?;
            let next = cron.next_after(since(scheduled))?;
            Some(if next <= now { now } else { next })
        }
    }
}

/// Runs are counted from the latest one, or from when the schedule was made.
fn since(scheduled: &Scheduled) -> Timestamp {
    scheduled.last.unwrap_or(scheduled.at).timestamp()
}

fn info(scheduled: &Scheduled, now: Timestamp) -> ScheduleInfo {
    let created = &scheduled.created;
    ScheduleInfo {
        schedule: created.schedule,
        when: created.when.clone(),
        prompt: created.prompt.clone(),
        goal: created.goal,
        budget: created.budget,
        created_at: scheduled.at,
        last: scheduled.last,
        next: next_run(scheduled, now).map(At::from),
    }
}
