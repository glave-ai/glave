//! Schedules: a prompt that runs at set times, once (`at`) or whenever a cron expression matches
//! on the clocks of the schedule's time zone, each run a new turn in the schedule's own chat.
//! Schedules live in their project's ledger. Making one makes its chat, named for it, where a
//! chat in its project works: on a copy of the project's folder where the folder is a
//! repository, so its runs never change files under the person, and otherwise where any chat
//! there works. Pausing one stops its runs until it is resumed; deleting or archiving its chat
//! deletes or pauses it; deleting it keeps its chat, with its past runs.
//! Firing them is `fire.rs`; how one reads in lists, `info.rs`.

mod fire;
mod info;
mod placed;

pub(crate) use placed::Placed;

use glave_event::{
    Principal, ScheduleCreated, ScheduleDeleted, ScheduleId, SchedulePaused, ThreadId, When,
};
use glave_harness::Change;
use glave_ports::{
    BlobStore, Caller, EventLog, Located, ModelProvider, ProjectId, Scope, Standing, title_from,
};

use crate::choice::Choice;
use crate::cron::Cron;
use crate::protocol::{ApiError, Reply, ScheduleInfo, code};
use crate::server::Server;
use crate::words::Sentence;
use crate::zone::time_zone;

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
    /// Whether schedules run here: a server on the store fires them, and policy lets a person
    /// keep one.
    pub(crate) fn schedules_run(&self) -> bool {
        self.inner.schedules && self.inner.limits.max_schedules != Some(0)
    }

    /// Makes the schedule `new` asks for, and its chat, for `caller`: refused where no server
    /// fires schedules.
    pub(crate) async fn create_schedule(
        &self,
        caller: &Caller,
        new: NewSchedule,
    ) -> Result<ScheduleInfo, ApiError> {
        if !self.inner.schedules {
            return Err(ApiError::new(
                code::NOT_AVAILABLE,
                Sentence::SchedulesNotHere,
            ));
        }
        if let When::Cron(text) = &new.when {
            Cron::for_a_schedule(text).map_err(|why| ApiError::new(code::INVALID_PARAMS, why))?;
        }
        if new.budget == 0 {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoBudget,
            ));
        }
        // Each run starts with the prompt: a name alone runs nothing.
        let Some(first) = title_from(&new.prompt) else {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoPrompt,
            ));
        };
        let name = new.name.as_deref().and_then(title_from).unwrap_or(first);
        let zone = match new.zone {
            Some(zone) => zone.trim().to_owned(),
            None => self.zone_of(caller).await?,
        };
        time_zone(&zone)?;
        let (_, scope) = self.project_for(caller, new.project).await?;
        self.room_for_one_more(caller).await?;
        let now = self.inner.harness.clock().now();
        let schedule = ScheduleId::new(now, self.inner.harness.entropy().bytes()?)?;
        let thread = self.schedule_chat(caller, scope).await?;
        let created = ScheduleCreated {
            schedule,
            name,
            when: new.when,
            zone,
            prompt: new.prompt,
            budget: new.budget,
            thread: Some(thread),
            from: new.from,
            // Named always, never left implied: another server's own person may be someone else.
            by: Some(Principal::person(self.person_of(caller)?)),
        };
        let project = scope.located();
        if let Err(refused) = self.keep_schedule(caller, project, &created).await {
            // One whose record landed though keeping it failed goes, so none outlives its chat.
            self.forget_schedule(caller, project, schedule).await;
            // A chat made for a schedule that isn't kept goes, with its copy.
            if let Err(error) = self.change(caller, thread, Change::Delete).await {
                tracing::warn!(error = %error.message, %thread, "a schedule's chat was left after the schedule was not kept");
            }
            return Err(refused);
        }
        self.schedule_info(project, schedule)
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::ScheduleNotKept))
    }

    /// Names `created`'s chat for it, then keeps it in `project`'s ledger, for `caller`: its
    /// project listed before and after, so the server that fires schedules finds it once it is
    /// recorded, whatever it unplaced meanwhile; counted again once kept.
    async fn keep_schedule(
        &self,
        caller: &Caller,
        project: Located,
        created: &ScheduleCreated,
    ) -> Result<(), ApiError> {
        if let Some(thread) = created.thread {
            self.rename(caller, thread, created.name.clone()).await?;
        }
        self.room_for_one_more(caller).await?;
        self.place_schedules(project).await?;
        self.record_in_ledger(project, created).await?;
        self.place_schedules(project).await?;
        // Counted again once kept, so schedules made at once on any server never keep more than
        // policy allows: one that finds too many goes, to be made again; both may.
        if self.kept_schedules(caller).await? > self.most_schedules() {
            let deleted = ScheduleDeleted {
                schedule: created.schedule,
                by: self.someone_else(caller.person),
            };
            self.record_in_ledger(project, &deleted).await?;
            return Err(ApiError::new(code::LIMIT, Sentence::WrittenElsewhere));
        }
        Ok(())
    }

    /// Deletes `schedule` from `project`'s ledger where its record landed though keeping it
    /// failed, for `caller`, so no schedule outlives the chat made for it; best effort.
    async fn forget_schedule(&self, caller: &Caller, project: Located, schedule: ScheduleId) {
        let forgot = async {
            if !(self.ledger_apart(project).await?.schedules).contains_key(&schedule) {
                return Ok(());
            }
            let deleted = ScheduleDeleted {
                schedule,
                by: self.someone_else(caller.person),
            };
            self.record_in_ledger(project, &deleted).await
        };
        if let Err(error) = forgot.await {
            tracing::warn!(error = %error.message, %schedule, "a schedule not kept was left");
        }
    }

    /// A new chat in `scope` for a schedule's runs: on a copy of its project's folder where the
    /// folder is a git repository with a commit, else in the folder itself, or where any new
    /// chat there works when it has none. Any other reason a copy can't be made refuses it, so
    /// a repository's runs never land in the person's own folder.
    async fn schedule_chat(&self, caller: &Caller, scope: Scope) -> Result<ThreadId, ApiError> {
        let choice = Choice::default();
        match self.new_thread(caller, scope, None, true, &choice).await {
            Err(refused) if crate::tasks::is_no_copy(&refused) => {
                (self.new_thread(caller, scope, Some(false), false, &choice)).await
            }
            made => made,
        }
    }

    /// Reads the task ledger of the project `at` where anyone made a schedule, so the
    /// summaries of its chats say their schedules: on a best effort, as a summary of a chat
    /// whose schedule can't be read now says what was read before, or none, and the chat still
    /// reads.
    pub(crate) async fn read_schedules_in(&self, at: Located) {
        let read = async {
            if self.has_schedules(at).await? {
                self.catch_up(at).await?;
            }
            Ok::<_, ApiError>(())
        };
        if let Err(error) = read.await {
            let project = at.project;
            tracing::warn!(%error, %project, "could not read the project's schedules");
        }
    }

    /// The schedule `chat` is the chat of, in the project `at`, as lists show it: as far as
    /// this server has read the project's ledger.
    pub(crate) fn schedule_of_chat(&self, at: Located, chat: ThreadId) -> Option<ScheduleInfo> {
        let now = self.inner.harness.clock().now();
        let ledgers = self.ledgers();
        let ledger = ledgers.get(&at)?;
        let scheduled =
            (ledger.schedules.values()).find(|scheduled| scheduled.created.thread == Some(chat))?;
        Some(info::info(at, scheduled, ledger, now))
    }

    /// Refused once `caller` keeps as many schedules as policy allows that will still run, or
    /// may once resumed, a hundred where it sets none: a paused one counts, so pausing makes
    /// no room.
    async fn room_for_one_more(&self, caller: &Caller) -> Result<(), ApiError> {
        let kept = self.kept_schedules(caller).await?;
        if kept >= self.most_schedules() {
            return Err(self.too_many_schedules(kept));
        }
        Ok(())
    }

    /// The schedules `caller` keeps that will still run, or may once resumed, read whole from
    /// the ledger of each project of theirs the index lists, apart from what this server keeps:
    /// so one made on another server a moment ago counts.
    async fn kept_schedules(&self, caller: &Caller) -> Result<usize, ApiError> {
        // The list read up to now; each project of theirs looked up in it.
        self.placed().await;
        let theirs = self.projects_of(caller).await?;
        let now = self.inner.harness.clock().now();
        let mut kept = 0usize;
        for project in theirs.into_iter().filter(|at| self.is_placed(*at)) {
            let ledger = self.ledger_apart(project).await?;
            let here = (ledger.schedules.values()).filter(|scheduled| info::kept(scheduled, now));
            kept = kept.saturating_add(here.count());
        }
        Ok(kept)
    }

    /// `kept` schedules are as many as policy allows, or more, said in plain words.
    fn too_many_schedules(&self, kept: usize) -> ApiError {
        let most = self.most_schedules();
        let said = match self.inner.limits.max_schedules {
            Some(_) => Sentence::TooManySchedules { kept, most },
            None => Sentence::TooManySchedulesKept { kept, most },
        };
        ApiError::new(code::LIMIT, said)
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
                (ledger.schedules.values())
                    .map(|scheduled| info::info(*project, scheduled, ledger, now))
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
        if !paused {
            self.may_resume(project, schedule).await?;
        }
        self.set_paused(caller, project, schedule, paused).await?;
        let schedule = (self.schedule_info(project, schedule))
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::ScheduleNotKept))?;
        Ok(Reply::Schedule { schedule })
    }

    /// Refuses resuming `schedule` of `project` while its chat is archived, and one made before
    /// schedules had chats, which runs no more.
    async fn may_resume(&self, project: Located, schedule: ScheduleId) -> Result<(), ApiError> {
        let chat = (self.ledgers().get(&project))
            .and_then(|ledger| ledger.schedules.get(&schedule))
            .map(|scheduled| scheduled.created.thread);
        let Some(Some(chat)) = chat else {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::ScheduleNoChatToResume,
            ));
        };
        if self.standing(chat).await? == Standing::Archived {
            return Err(ApiError::new(
                code::ARCHIVED,
                Sentence::ScheduleChatArchived,
            ));
        }
        Ok(())
    }

    /// Records `schedule` of `project` paused (`paused`) or resumed for `caller`, unless it is
    /// so already.
    async fn set_paused(
        &self,
        caller: &Caller,
        project: Located,
        schedule: ScheduleId,
        paused: bool,
    ) -> Result<(), ApiError> {
        let already = (self.ledgers().get(&project))
            .and_then(|ledger| ledger.schedules.get(&schedule))
            .is_some_and(|scheduled| scheduled.paused == paused);
        if !already {
            let by = self.someone_else(caller.person);
            let changed = SchedulePaused {
                schedule,
                paused,
                by,
            };
            self.record_in_ledger(project, &changed).await?;
        }
        Ok(())
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

    /// Follows `thread`'s change to the schedule whose chat it is, if one is, for `caller`: a
    /// schedule runs only into its own chat, so deleting the chat deletes it, and archiving the
    /// chat pauses it.
    pub(crate) async fn chat_put_away(
        &self,
        caller: &Caller,
        scope: &Scope,
        thread: ThreadId,
        deleted: bool,
    ) -> Result<(), ApiError> {
        let project = scope.located();
        self.catch_up(project).await?;
        let theirs = (self.ledgers().get(&project)).and_then(|ledger| {
            (ledger.schedules.values())
                .find(|scheduled| scheduled.created.thread == Some(thread))
                .map(|scheduled| scheduled.created.schedule)
        });
        let Some(schedule) = theirs else {
            return Ok(());
        };
        if !deleted {
            return self.set_paused(caller, project, schedule, true).await;
        }
        let deleted = ScheduleDeleted {
            schedule,
            by: self.someone_else(caller.person),
        };
        self.record_in_ledger(project, &deleted).await
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
            .ok_or_else(|| {
                ApiError::new(
                    code::NO_SCHEDULE,
                    Sentence::NoSchedule {
                        schedule: schedule.to_string(),
                    },
                )
            })
    }

    /// `schedule` of `project`, as lists show it, as this server last read its ledger.
    fn schedule_info(&self, project: Located, schedule: ScheduleId) -> Option<ScheduleInfo> {
        let now = self.inner.harness.clock().now();
        let ledgers = self.ledgers();
        let ledger = ledgers.get(&project)?;
        let scheduled = ledger.schedules.get(&schedule)?;
        Some(info::info(project, scheduled, ledger, now))
    }

    /// The projects `caller` works in, each one's ledger read up to now.
    async fn read_ledgers_of(&self, caller: &Caller) -> Result<Vec<Located>, ApiError> {
        let projects = self.projects_of(caller).await?;
        for project in &projects {
            self.catch_up(*project).await?;
        }
        Ok(projects)
    }
}
