//! Schedules: a prompt that runs at set times, once (`at`) or whenever a cron expression matches
//! on the clocks of the person's time zone, each run a new turn in the schedule's own chat.
//! Schedules live in their project's ledger. Making one makes its chat, named for it, on a copy
//! of the project's folder as every task's thread is; pausing one stops its runs until it is
//! resumed, and removing one leaves its chat with its past runs. Firing them is `fire.rs`.

mod fire;

use glave_event::{
    At, Principal, PrincipalId, ScheduleCreated, ScheduleDeleted, ScheduleId, SchedulePaused,
    ScheduleResumed, ThreadId, When,
};
use glave_ports::{BlobStore, Caller, EventLog, Located, ModelProvider, ProjectId, title_from};
use jiff::Timestamp;

use crate::choice::Choice;
use crate::cron::Cron;
use crate::ledger::{Ledger, Recorded, Scheduled};
use crate::protocol::{ApiError, LastRun, Reply, ScheduleInfo, code};
use crate::repeats::repeats;
use crate::server::Server;
use crate::words::Sentence;
use crate::zone::time_zone;

/// Occurrences of a cron expression walked to find the latest one missed.
const MOST_MISSED: usize = 100_000;

/// What `schedule/create` asks for.
pub(crate) struct NewSchedule {
    pub(crate) project: Option<ProjectId>,
    pub(crate) name: Option<String>,
    pub(crate) when: When,
    /// Its time zone; the caller's when none is given.
    pub(crate) zone: Option<String>,
    pub(crate) prompt: String,
    pub(crate) budget: u64,
    /// The chat it is set up from, if it is.
    pub(crate) from: Option<ThreadId>,
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Makes the schedule `new` asks for, and its chat, for `caller`.
    pub(crate) async fn create_schedule(
        &self,
        caller: &Caller,
        new: NewSchedule,
    ) -> Result<ScheduleInfo, ApiError> {
        if let When::Cron(text) = &new.when {
            Cron::parse(text).map_err(|why| ApiError::new(code::INVALID_PARAMS, why))?;
        }
        if new.budget == 0 {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoBudget,
            ));
        }
        let name = (new.name.as_deref().and_then(title_from)).or_else(|| title_from(&new.prompt));
        let Some(name) = name else {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoPrompt,
            ));
        };
        let zone = match new.zone {
            Some(zone) => zone.trim().to_owned(),
            None => self.zone_of(caller).await?,
        };
        time_zone(&zone)?;
        let (_, scope) = self.project_for(caller, new.project).await?;
        let project = scope.located();
        self.room_for_one_more(caller).await?;
        // Its chat works on a copy of its own, as every task's thread does.
        let thread = (self.new_thread(caller, scope, None, true, &Choice::default())).await?;
        self.rename(caller, thread, name.clone()).await?;
        let now = self.inner.harness.clock().now();
        let schedule = ScheduleId::new(now, self.inner.harness.entropy().bytes()?)?;
        let created = ScheduleCreated {
            schedule,
            name,
            when: new.when,
            zone,
            prompt: new.prompt,
            budget: new.budget,
            thread: Some(thread),
            from: new.from,
            by: self.someone_else(caller.person),
        };
        self.record_in_ledger(project, &created).await?;
        self.schedule_info(project, schedule)
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::ScheduleNotKept))
    }

    /// Refused once `caller` keeps as many schedules as policy allows, paused ones among them.
    async fn room_for_one_more(&self, caller: &Caller) -> Result<(), ApiError> {
        let Some(most) = self.inner.limits.max_schedules else {
            return Ok(());
        };
        let projects = self.read_ledgers_of(caller).await?;
        let kept = self
            .ledgers()
            .iter()
            .filter(|(id, _)| projects.contains(id))
            .flat_map(|(_, ledger)| ledger.schedules.values())
            .filter(|scheduled| !ran_its_once(scheduled))
            .count();
        if kept >= most {
            return Err(ApiError::new(
                code::LIMIT,
                Sentence::TooManySchedules { kept, most },
            ));
        }
        Ok(())
    }

    /// The schedules of every project `caller` works in, oldest first.
    pub(crate) async fn list_schedules(
        &self,
        caller: &Caller,
    ) -> Result<Vec<ScheduleInfo>, ApiError> {
        let projects = self.read_ledgers_of(caller).await?;
        let now = self.inner.harness.clock().now();
        let mut schedules: Vec<ScheduleInfo> = self
            .ledgers()
            .iter()
            .filter(|(project, _)| projects.contains(project))
            .flat_map(|(project, ledger)| {
                (ledger.schedules.values()).map(|scheduled| info(*project, scheduled, ledger, now))
            })
            .collect();
        schedules.sort_by_key(|schedule| schedule.schedule);
        Ok(schedules)
    }

    /// Pauses `schedule` for `caller` (`paused`), or resumes it; one already so answers as it
    /// is.
    pub(crate) async fn pause_schedule(
        &self,
        caller: &Caller,
        schedule: ScheduleId,
        paused: bool,
    ) -> Result<Reply, ApiError> {
        let project = self.project_of_schedule(caller, schedule).await?;
        let already = (self.ledgers().get(&project))
            .and_then(|ledger| ledger.schedules.get(&schedule))
            .is_some_and(|scheduled| scheduled.paused == paused);
        let by = self.someone_else(caller.person);
        match (already, paused) {
            (true, _) => {}
            (false, true) => {
                let paused = SchedulePaused { schedule, by };
                self.record_in_ledger(project, &paused).await?;
            }
            (false, false) => {
                let resumed = ScheduleResumed { schedule, by };
                self.record_in_ledger(project, &resumed).await?;
            }
        }
        let schedule = self
            .schedule_info(project, schedule)
            .ok_or_else(|| no_schedule(schedule))?;
        Ok(Reply::Schedule { schedule })
    }

    /// Removes `schedule` for `caller`: it starts no more runs, and its chat stays.
    pub(crate) async fn delete_schedule(
        &self,
        caller: &Caller,
        schedule: ScheduleId,
    ) -> Result<Reply, ApiError> {
        let project = self.project_of_schedule(caller, schedule).await?;
        let deleted = ScheduleDeleted {
            schedule,
            by: self.someone_else(caller.person),
        };
        self.record_in_ledger(project, &deleted).await?;
        Ok(Reply::Done {})
    }

    /// The project of `caller`'s that keeps `schedule`, its ledger read up to now.
    async fn project_of_schedule(
        &self,
        caller: &Caller,
        schedule: ScheduleId,
    ) -> Result<Located, ApiError> {
        let projects = self.read_ledgers_of(caller).await?;
        self.ledgers()
            .iter()
            .find(|(project, ledger)| {
                projects.contains(project) && ledger.schedules.contains_key(&schedule)
            })
            .map(|(project, _)| *project)
            .ok_or_else(|| no_schedule(schedule))
    }

    /// `schedule` of `project`, as lists show it, as this server last read its ledger.
    fn schedule_info(&self, project: Located, schedule: ScheduleId) -> Option<ScheduleInfo> {
        let now = self.inner.harness.clock().now();
        let ledgers = self.ledgers();
        let ledger = ledgers.get(&project)?;
        let scheduled = ledger.schedules.get(&schedule)?;
        Some(info(project, scheduled, ledger, now))
    }

    /// The projects `caller` works in, each one's ledger read up to now.
    async fn read_ledgers_of(&self, caller: &Caller) -> Result<Vec<Located>, ApiError> {
        let projects = self.projects_of(caller).await?;
        for project in &projects {
            self.catch_up(*project).await?;
        }
        Ok(projects)
    }

    /// `person` as a schedule's events record them: absent for the server's own person.
    fn someone_else(&self, person: Option<PrincipalId>) -> Option<Principal> {
        person
            .filter(|person| Some(*person) != self.inner.person)
            .map(Principal::person)
    }
}

/// Refused: no schedule `schedule` among the caller's.
fn no_schedule(schedule: ScheduleId) -> ApiError {
    ApiError::new(
        code::NO_SCHEDULE,
        Sentence::NoSchedule {
            schedule: schedule.to_string(),
        },
    )
}

/// The time a run of `scheduled` is due by `now`, if one is: for a cron schedule that missed
/// several, the latest of them. None while it is paused.
fn due(scheduled: &Scheduled, now: Timestamp) -> Option<Timestamp> {
    if scheduled.paused {
        return None;
    }
    match &scheduled.created.when {
        When::At(at) => {
            let waits = scheduled.last.or(scheduled.held).is_none();
            (waits && at.timestamp() <= now).then(|| at.timestamp())
        }
        When::Cron(text) => {
            let cron = Cron::parse(text).ok()?;
            let zone = time_zone(&scheduled.created.zone).ok()?;
            let mut at = cron.next_after(since(scheduled), &zone)?;
            if at > now {
                return None;
            }
            for _ in 0..MOST_MISSED {
                match cron.next_after(at, &zone) {
                    Some(later) if later <= now => at = later,
                    _ => return Some(at),
                }
            }
            Some(now)
        }
    }
}

/// When `scheduled` runs next after `now`, or is due now; `None` once a one-off has run, and
/// while it is paused.
fn next_run(scheduled: &Scheduled, now: Timestamp) -> Option<Timestamp> {
    if scheduled.paused || ran_its_once(scheduled) {
        return None;
    }
    let next = match &scheduled.created.when {
        When::At(at) => at.timestamp(),
        When::Cron(text) => {
            let zone = time_zone(&scheduled.created.zone).ok()?;
            Cron::parse(text)
                .ok()?
                .next_after(since(scheduled), &zone)?
        }
    };
    Some(next.max(now))
}

/// Whether `scheduled` is a one-off that has run: it runs no more.
fn ran_its_once(scheduled: &Scheduled) -> bool {
    matches!(scheduled.created.when, When::At(_)) && scheduled.last.is_some()
}

/// Runs are counted from the latest one, or the latest run held back, or from when the schedule
/// was resumed or made.
fn since(scheduled: &Scheduled) -> Timestamp {
    (scheduled.last.max(scheduled.held).max(scheduled.resumed))
        .unwrap_or(scheduled.at)
        .timestamp()
}

fn info(project: Located, scheduled: &Scheduled, ledger: &Ledger, now: Timestamp) -> ScheduleInfo {
    let created = &scheduled.created;
    ScheduleInfo {
        schedule: created.schedule,
        name: created.name.clone(),
        project: project.project,
        thread: created.thread,
        from: created.from,
        when: created.when.clone(),
        zone: created.zone.clone(),
        repeats: repeats(&created.when, true).into(),
        prompt: created.prompt.clone(),
        budget: created.budget,
        created_at: scheduled.at,
        paused: scheduled.paused,
        last: last_run(scheduled, ledger),
        next: next_run(scheduled, now).map(At::from),
    }
}

/// What came of `scheduled`'s latest run, as `ledger` has it.
fn last_run(scheduled: &Scheduled, ledger: &Ledger) -> Option<LastRun> {
    let due = scheduled.last?;
    if scheduled.skipped {
        return Some(LastRun::Skipped { due });
    }
    let schedule = scheduled.created.schedule;
    let started = |task: &&Recorded| {
        (task.started.schedule).is_some_and(|fired| fired.schedule == schedule && fired.due == due)
    };
    if ledger.open.values().any(|task| started(&task)) {
        return Some(LastRun::Running { due });
    }
    let ended = ledger.finished.iter().rev().find(started);
    Some(match ended.and_then(|task| task.ended.as_ref()) {
        Some((ended, _)) => LastRun::Ended {
            due,
            outcome: ended.outcome,
        },
        None => LastRun::Started { due },
    })
}
