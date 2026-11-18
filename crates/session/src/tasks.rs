//! Tasks: a turn run in a thread with no one attached, in any project its caller works in,
//! always on a copy of the project's folder (a worktree or a scratch folder), never in the
//! folder itself, but for a schedule's run, which works where its chat does. The server starts
//! the work as it would for a client, answers its approvals "no one there" (in auto mode the
//! judge still clears what the person asked for) but for a schedule's run, which waits for the
//! person as their chat does, keeps a running tally of its tokens and cost from the `model.responded` events
//! it records, stops a turn at its budget, and records the task's start and end in its project's
//! ledger. `tasks/list` and `tasks/stop` show and stop them; `task/finished` tells the thread's
//! watchers when one ends.

mod acting;
mod adopted;
mod run;

pub(crate) use acting::{Membership, PeopleOnce};
pub(crate) use run::Unstarted;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use glave_event::{
    At, Body, Currency, Due, Money, Nanos, Opened, Outcome, Payload, Principal, PrincipalId,
    TaskEnded, TaskStarted, ThreadId, TokenCounts, WorkspaceBound, decode,
};
use glave_harness::Cancel;
use glave_ports::computer::{Reason, Refusal};
use glave_ports::{
    BlobStore, Caller, EventLog, Lease, Located, ModelProvider, OrgId, Scope, title_from,
};

use crate::choice::Choice;
use crate::held::{Slot, busy};
use crate::leased::{Repaired, elsewhere, renewing};
use crate::ledger::Recorded;
use crate::protocol::{
    ApiError, Notification, Reply, TaskInfo, TaskKind, TaskStatus, TurnResult, code,
};
use crate::server::Server;
use crate::steer::{Message, Sent};
use crate::turn::{TurnItem, TurnStream};
use crate::words::Sentence;

/// What a person may run at once and keep scheduled (`[tasks]` in `policy.toml`): with no rule,
/// four tasks at once and a hundred schedules that will still run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskLimits {
    pub max_running: Option<usize>,
    pub max_schedules: Option<usize>,
}

/// The tasks a person runs at once where policy sets no `max_running`, decided: each is a copy
/// of a folder, model calls and commands on one computer (`specs/one-server.md`).
const MOST_RUNNING: usize = 4;

/// The schedules a person keeps that will still run where policy sets no `max_schedules`: each
/// is runs of theirs at set times, read every tick.
const MOST_SCHEDULES: usize = 100;

/// What a task does: one turn with this prompt, with the model and mode chosen with it, if any.
pub(crate) struct Work(pub(crate) String, pub(crate) Choice);

/// A task running in this server.
#[derive(Debug, Clone)]
pub(crate) struct Running {
    /// The project whose ledger records it.
    project: Located,
    started: TaskStarted,
    at: At,
    by: Option<PrincipalId>,
    tokens: TokenCounts,
    cost: Option<Money>,
    /// Whether someone stopped it, and who, when the server knows.
    stopped: bool,
    stopped_by: Option<PrincipalId>,
    over_budget: bool,
    /// Stops following it, where it is followed by what its thread records: as it is stopped,
    /// as the server stops, or as another server takes its task lease.
    unfollowed: Cancel,
}

pub(crate) type Tasks = HashMap<ThreadId, Running>;

/// Refused: a task asked for in the folder itself.
pub(crate) fn in_place() -> ApiError {
    ApiError::new(code::INVALID_PARAMS, Sentence::TaskInPlace)
}

/// Refused: no copy of the project's folder could be made for a task to work on, since the
/// folder is no git repository with a commit.
pub(crate) fn no_copy() -> ApiError {
    ApiError::new(code::WORKSPACE, Sentence::TaskNoCopy)
}

/// Whether `refused` is [`no_copy`]'s, and not any other reason a copy could not be made.
pub(crate) fn is_no_copy(refused: &ApiError) -> bool {
    refused.code == code::WORKSPACE && refused.message.is(&Sentence::TaskNoCopy)
}

/// Whether a computer refused a copy for `reason` as the folder is no git repository with a
/// commit to copy.
pub(crate) fn no_repository(reason: &Reason) -> bool {
    matches!(
        reason.refusal(),
        Some(Refusal::NotInRepository { .. } | Refusal::NoCommitsYet { .. })
    )
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The scope of `thread`, refused for a task for `caller` where the thread works in its
    /// project's folder itself, beyond the policy's limit, or while a turn runs there.
    async fn may_start_task(
        &self,
        caller: &Caller,
        thread: ThreadId,
        run: bool,
    ) -> Result<Scope, ApiError> {
        let scope = self.authorize(caller, thread).await?;
        // A schedule's run works where its chat was made to.
        if !run && self.in_its_folder(&scope, thread).await? {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::TaskThreadInFolder,
            ));
        }
        let (running, most) = (self.running_for(caller.person), self.most_running());
        if running >= most {
            let said = match self.inner.limits.max_running {
                Some(_) => Sentence::TooManyTasks { running, most },
                None => Sentence::TooManyTasksAtOnce { running, most },
            };
            return Err(ApiError::new(code::LIMIT, said));
        }
        // Checked before no one answers for the thread, so a turn someone follows keeps its
        // answers.
        let busy = matches!(self.lock().get(&thread), Some(Slot::Busy { .. }));
        if busy || self.tasks().contains_key(&thread) {
            return Err(crate::held::busy(thread));
        }
        Ok(scope)
    }

    /// Whether `thread` works in a folder itself, as its latest binding says; one with no
    /// binding works in its project's folder.
    async fn in_its_folder(&self, scope: &Scope, thread: ThreadId) -> Result<bool, ApiError> {
        let latest = self.latest(scope, thread, WorkspaceBound::TYPE).await?;
        Ok(match latest {
            Some(Body::WorkspaceBound(bound)) => matches!(bound.opened, Opened::Folder { .. }),
            _ => true,
        })
    }

    /// Starts `work` in `thread` as a task for `caller`, and answers with it at once.
    pub(crate) async fn start_task(
        &self,
        caller: &Caller,
        thread: ThreadId,
        work: Work,
        budget: Option<u64>,
    ) -> Result<TaskInfo, ApiError> {
        let project = (self.may_start_task(caller, thread, false).await?).located();
        self.run_task(caller, (project, thread), work, budget, None)
            .await
    }

    /// The task lease of `thread` in `org`: `held`, where a schedule's claim took it already,
    /// else taken now; refused while another server holds it.
    async fn task_lease(
        &self,
        held: Option<Lease>,
        org: OrgId,
        thread: ThreadId,
    ) -> Result<Lease, ApiError> {
        match held {
            Some(lease) => Ok(lease),
            None => (self.take_task(org, thread).await?).ok_or_else(|| elsewhere(thread)),
        }
    }

    /// Starts `work` in `thread` of `project` as a task for `caller`, who may, and answers
    /// with it at once: its start recorded once its turn started, unless it came `claimed`, a
    /// schedule's run whose start was recorded first under its task's lease (`run.rs`).
    async fn run_task(
        &self,
        caller: &Caller,
        (project, thread): (Located, ThreadId),
        work: Work,
        budget: Option<u64>,
        claimed: Option<(TaskStarted, Lease)>,
    ) -> Result<TaskInfo, ApiError> {
        let (claimed, held) = claimed.unzip();
        let schedule = claimed.as_ref().and_then(|started| started.schedule);
        // Whoever follows a task holds its task lease, so one server records its end.
        let lease = self.task_lease(held, project.org, thread).await?;
        let Work(prompt, choice) = work;
        let title = title_from(&prompt).unwrap_or_default();
        // A schedule's run waits for the person where it needs them, as their chat does; no one
        // answers any other task.
        if schedule.is_none() {
            self.inner.approvals.unattend(thread);
        }
        // A task's own turn never queues: one running in the thread refuses it.
        let message = Message {
            files: &[],
            prompt: &prompt,
            choice,
            queue: false,
            // Its thread is unattended for the whole task already, where it is.
            unattended: false,
            scheduled: schedule.map(|fired| Due {
                zone: self.zone_of_schedule(project, fired.schedule),
                at: fired.due,
            }),
        };
        let stream = self
            .start_turn_with(caller, thread, message, None)
            .await
            .and_then(|sent| match sent {
                Sent::Started(stream) => Ok(stream),
                Sent::Queued(_) => Err(busy(thread)),
            });
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                self.inner.approvals.attend(thread);
                self.let_go(&lease).await;
                return Err(error);
            }
        };
        // Its end is what the ledger records, though its person signs out meanwhile.
        stream.outlasting_access();
        let started = claimed.clone().unwrap_or_else(|| TaskStarted {
            thread,
            title,
            model: stream.model.clone(),
            budget,
            schedule,
            by: self.someone_else(caller.person),
        });
        let running = Running {
            project,
            started: started.clone(),
            at: At::from(self.inner.harness.clock().now()),
            by: caller.person,
            tokens: TokenCounts::default(),
            cost: Some(Money {
                nanos: Nanos::ZERO,
                currency: Currency::USD,
            }),
            stopped: false,
            stopped_by: None,
            over_budget: false,
            unfollowed: Cancel::new(),
        };
        let info = self.running_info(&running);
        let unfollowed = running.unfollowed.clone();
        self.tasks().insert(thread, running);
        let recorded = match claimed {
            Some(_) => Ok(()),
            None => self.record_in_ledger(project, &started).await,
        };
        if let Err(error) = recorded {
            self.cancel_busy(thread);
            self.follow_task(thread, stream).await;
            self.let_go(&lease).await;
            return Err(error);
        }
        let server = self.clone();
        self.follow_held(lease, unfollowed, async move {
            server.follow_task(thread, stream).await;
        });
        Ok(info)
    }

    /// Runs `follow`, which follows a task to its end, in the background under the task's
    /// `lease`, renewed as it goes and let go of once it ends; lost to another server, which
    /// follows the task from then, `unfollowed` stops a follower of its thread's records.
    fn follow_held(
        &self,
        lease: Lease,
        unfollowed: Cancel,
        follow: impl Future<Output = ()> + Send + 'static,
    ) {
        let server = self.clone();
        let task = tokio::spawn(async move {
            let leases = Arc::clone(&server.inner.leases);
            renewing(leases.as_ref(), &lease, &unfollowed, follow).await;
            server.let_go(&lease).await;
        });
        self.own(task);
    }

    /// Follows a task's work to its end, then records how it ended and says so.
    async fn follow_task(&self, thread: ThreadId, mut stream: TurnStream) {
        let mut ended = loop {
            match stream.next().await {
                Some(TurnItem::Note(note)) => {
                    self.count(thread, &note);
                    // Its turn's own end, whoever went on with it after a wait.
                    if let Some(result) = turn_ended(thread, &note) {
                        break Ok(result);
                    }
                }
                // A watcher this far behind is rare; the turn's own totals settle it at the end.
                Some(TurnItem::Missed(_)) => {}
                Some(TurnItem::Ended(ended)) => break ended,
                None => break Err(ApiError::new(code::INTERNAL, Sentence::TaskStopped)),
            }
        };
        // A turn waiting for the person whose follower was let go, but as this server stops: it
        // is followed by what its thread records, to its end.
        if matches!(&ended, Err(refused) if refused.code == code::NOT_AVAILABLE)
            && !self.is_stopping()
        {
            ended = self.follow_by_notes(thread).await;
        }
        self.task_ended(thread, ended).await;
    }

    /// Records how `thread`'s task `ended` and says so; one this server stops following
    /// (`-32022`) and no one stopped is let go of: as it stops, it stays open in the ledger for
    /// the next server to adopt, and one whose task lease another server took is that server's.
    async fn task_ended(&self, thread: ThreadId, ended: Result<TurnResult, ApiError>) {
        self.inner.approvals.attend(thread);
        let Some(running) = self.tasks().remove(&thread) else {
            return;
        };
        self.inner.tasks_ended.notify_one();
        let let_go = matches!(&ended, Err(refused) if refused.code == code::NOT_AVAILABLE);
        if let_go && !running.stopped {
            return;
        }
        let ended = settle(&running, &ended, self.is_stopping());
        // Recorded on a best effort: the thread's own events hold the work either way.
        let _ = self.record_in_ledger(running.project, &ended).await;
        let now = At::from(self.inner.harness.clock().now());
        let recorded = Recorded {
            started: running.started,
            at: running.at,
            ended: Some((ended, now)),
        };
        let task = self.recorded_info(running.project, &recorded, TaskStatus::Finished);
        self.inner
            .hub
            .publish(thread, Notification::TaskFinished { thread, task });
    }

    /// Adds a model call the task recorded (its own or a subagent's) to its tally, and stops a
    /// turn that has reached its budget.
    fn count(&self, thread: ThreadId, note: &Notification) {
        let Notification::Event { event, .. } = note else {
            return;
        };
        if event.kind.as_str() != "model.responded" {
            return;
        }
        let Ok(decoded) = decode(event.clone()) else {
            return;
        };
        let Body::ModelResponded(responded) = decoded.body else {
            return;
        };
        let over = {
            let mut tasks = self.tasks();
            let Some(running) = tasks.get_mut(&thread) else {
                return;
            };
            running.tokens += responded.usage.counts();
            running.cost = running
                .cost
                .zip(responded.cost)
                .and_then(|(sum, cost)| sum.checked_add(cost.amount));
            let spent = running.tokens.input.saturating_add(running.tokens.output);
            let over = !running.over_budget
                && running.started.budget.is_some_and(|budget| spent >= budget);
            running.over_budget |= over;
            over
        };
        if over {
            self.cancel_busy(thread);
        }
    }

    /// Stops the task running in `thread` for `caller`, and waits until it has ended.
    pub(crate) async fn stop_task(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Reply, ApiError> {
        let unfollowed = {
            let mut tasks = self.tasks();
            let Some(running) = tasks.get_mut(&thread) else {
                return Err(ApiError::new(
                    code::NO_TURN,
                    Sentence::NoTask {
                        thread: thread.to_string(),
                    },
                ));
            };
            running.stopped = true;
            running.stopped_by = caller.person.or(self.inner.person);
            running.unfollowed.clone()
        };
        // A schedule's run waiting for the person holds no slot: it goes on stopped, declining
        // what it waits for, as `turn/cancel` stops it.
        let stopped = match self.cancel(thread).await {
            // Its turn ended meanwhile, and its end is being recorded.
            Err(refused) if refused.code == code::NO_TURN => Ok(Reply::Done {}),
            stopped => stopped,
        };
        // One followed by its thread's records whose turn's end never came ends now.
        unfollowed.cancel();
        stopped
    }

    /// Records that `thread`'s task, if it runs one, is stopped by `by`, unless someone
    /// stopped it first; stopping what runs is the caller's.
    pub(crate) fn task_stopped_by(&self, thread: ThreadId, by: Option<PrincipalId>) {
        if let Some(running) = self.tasks().get_mut(&thread)
            && !running.stopped
        {
            running.stopped = true;
            running.stopped_by = by;
        }
    }

    /// Stops `thread`'s task, if it runs one, before it is archived or deleted.
    pub(crate) async fn stop_task_if_any(&self, caller: &Caller, thread: ThreadId) {
        if self.tasks().contains_key(&thread) {
            let _ = self.stop_task(caller, thread).await;
        }
    }

    /// The tasks of every project `caller` works in: running here, then running elsewhere, then
    /// recently finished, each newest first.
    pub(crate) async fn list_tasks(
        &self,
        caller: &Caller,
        limit: Option<usize>,
    ) -> Result<Reply, ApiError> {
        let projects = self.projects_of(caller).await?;
        for project in &projects {
            self.catch_up(*project).await?;
        }
        let limit = limit.unwrap_or(20).clamp(1, 500);
        let (mut running, mut elsewhere, mut finished) = {
            let ledgers = self.ledgers();
            let here = self.tasks();
            let running: Vec<TaskInfo> = here
                .values()
                .filter(|task| projects.contains(&task.project))
                .map(|task| self.running_info(task))
                .collect();
            let theirs: Vec<_> = projects
                .iter()
                .filter_map(|project| ledgers.get(project).map(|ledger| (*project, ledger)))
                .collect();
            let elsewhere: Vec<TaskInfo> = theirs
                .iter()
                .flat_map(|(project, ledger)| ledger.open.values().map(|task| (*project, task)))
                .filter(|(_, task)| !here.contains_key(&task.started.thread))
                .map(|(project, task)| self.recorded_info(project, task, TaskStatus::Elsewhere))
                .collect();
            let finished: Vec<(Option<At>, TaskInfo)> = theirs
                .iter()
                .flat_map(|(project, ledger)| ledger.finished.iter().map(|task| (*project, task)))
                .map(|(project, task)| {
                    let ended = task.ended.as_ref().map(|(_, at)| *at);
                    (
                        ended,
                        self.recorded_info(project, task, TaskStatus::Finished),
                    )
                })
                .collect();
            (running, elsewhere, finished)
        };
        running.sort_by_key(|task| std::cmp::Reverse(task.started_at));
        elsewhere.sort_by_key(|task| std::cmp::Reverse(task.started_at));
        finished.sort_by_key(|(ended, _)| std::cmp::Reverse(*ended));
        running.extend(elsewhere);
        running.extend(finished.into_iter().map(|(_, task)| task));
        running.truncate(limit);
        Ok(Reply::Tasks { tasks: running })
    }

    /// Records as interrupted every task a ledger has open that no server runs any more, in
    /// every project whose tasks this server runs: those whose thread's lease lapsed or that
    /// have none, since a task's turn holds its thread's lease wherever it runs. For the one
    /// server that runs tasks, as it starts, and again while another server's lease holds one
    /// (`Repaired::held`), until it lapses.
    pub async fn recover_tasks(&self) -> Result<Repaired, ApiError> {
        let mut recovered = Repaired::default();
        // The person's own projects with schedules made before where schedules are was kept
        // are listed, so the server firing schedules finds them wherever it runs; and any a
        // scheduler that stopped mid-unplace may have left out.
        self.placed().await;
        self.look_again_at_unplaced().await;
        for project in self.task_projects_here().await? {
            self.catch_up(project).await?;
            let any = (self.ledgers().get(&project)).is_some_and(|l| !l.schedules.is_empty());
            if any && !self.is_placed(project) {
                self.place_schedules(project).await?;
            }
        }
        for project in self.task_projects().await? {
            self.catch_up(project).await?;
            let here = self.recover_in(project, &PeopleOnce::new()).await?;
            recovered.closed = recovered.closed.saturating_add(here.closed);
            recovered.held = recovered.held.saturating_add(here.held);
        }
        Ok(recovered)
    }

    /// Recovers the tasks `project`'s ledger, as read, has open that no server follows any more:
    /// those whose task lease lapsed, adopted when a schedule's run waits for the person, else
    /// recorded interrupted. Those another server's lease holds are counted, to look at again.
    pub(crate) async fn recover_in(
        &self,
        project: Located,
        people: &PeopleOnce,
    ) -> Result<Repaired, ApiError> {
        let mut recovered = Repaired::default();
        {
            let stale: Vec<ThreadId> = {
                let ledgers = self.ledgers();
                let here = self.tasks();
                ledgers.get(&project).map_or_else(Vec::new, |ledger| {
                    ledger
                        .open
                        .keys()
                        .filter(|thread| !here.contains_key(thread))
                        .copied()
                        .collect()
                })
            };
            for thread in stale {
                // Whoever follows a task holds its task lease: one server recovers each.
                let Some(task) = self.take_task(project.org, thread).await? else {
                    recovered.held = recovered.held.saturating_add(1);
                    continue;
                };
                let Some(lease) = self.take(project.org, thread).await? else {
                    self.let_go(&task).await;
                    recovered.held = recovered.held.saturating_add(1);
                    continue;
                };
                // A schedule's run that waits for the person goes on, under its budget, once
                // they answer; one that can't be adopted is recorded interrupted.
                let adopted = match self.adopt_waiting(project, thread, task, people).await {
                    Ok(adopted) => adopted,
                    Err(error) => {
                        tracing::warn!(%error, %thread, "a waiting run could not be adopted");
                        false
                    }
                };
                if adopted {
                    self.let_go(&lease).await;
                    continue;
                }
                let ended = self.interrupt(project, thread).await;
                self.let_go(&lease).await;
                self.let_go(&task).await;
                if ended? {
                    recovered.closed = recovered.closed.saturating_add(1);
                }
            }
        }
        Ok(recovered)
    }

    /// Records `thread`'s task in `project` interrupted, under its thread's lease, unless its
    /// ending was recorded meanwhile; says whether it recorded.
    async fn interrupt(&self, project: Located, thread: ThreadId) -> Result<bool, ApiError> {
        self.catch_up(project).await?;
        let open = (self.ledgers().get(&project))
            .and_then(|ledger| ledger.open.get(&thread).map(|task| task.started.schedule));
        let Some(fired) = open else {
            return Ok(false);
        };
        let ended = TaskEnded {
            thread,
            due: fired.map(|fired| fired.due),
            outcome: Outcome::Interrupted,
            tokens: TokenCounts::default(),
            cost: None,
            by: None,
        };
        self.record_in_ledger(project, &ended).await?;
        Ok(true)
    }

    pub(crate) fn tasks(&self) -> std::sync::MutexGuard<'_, Tasks> {
        self.inner
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `person` as a task's or a schedule's events record them: absent for the server's own
    /// person.
    pub(crate) fn someone_else(&self, person: Option<PrincipalId>) -> Option<Principal> {
        person
            .filter(|person| Some(*person) != self.inner.person)
            .map(Principal::person)
    }

    /// The tasks one person may run at once here: policy's `max_running`, else four.
    pub(crate) fn most_running(&self) -> usize {
        self.inner.limits.max_running.unwrap_or(MOST_RUNNING)
    }

    /// The schedules one person may keep that will still run: policy's `max_schedules`, else
    /// a hundred.
    pub(crate) fn most_schedules(&self) -> usize {
        self.inner.limits.max_schedules.unwrap_or(MOST_SCHEDULES)
    }

    /// How many tasks of `person`'s this server runs now: none for the server's own person.
    pub(crate) fn running_for(&self, person: Option<PrincipalId>) -> usize {
        self.working(|task| task.by == person)
    }

    /// How many of the tasks `counted` takes are working now: a schedule's run waiting for the
    /// person holds nothing until they answer, and counts toward no limit.
    pub(crate) fn working(&self, counted: impl Fn(&Running) -> bool) -> usize {
        let threads: Vec<ThreadId> = (self.tasks().iter())
            .filter(|(_, task)| counted(task))
            .map(|(thread, _)| *thread)
            .collect();
        let held = self.lock();
        (threads.iter())
            .filter(|thread| matches!(held.get(thread), Some(Slot::Busy { .. })))
            .count()
    }

    /// Cancels what runs in `thread`, if anything does; what to wait on for it to end.
    fn cancel_busy(&self, thread: ThreadId) -> Option<tokio::sync::watch::Receiver<bool>> {
        match self.lock().get(&thread) {
            Some(Slot::Busy {
                cancel, finished, ..
            }) => {
                cancel.cancel();
                Some(finished.clone())
            }
            _ => None,
        }
    }

    fn running_info(&self, task: &Running) -> TaskInfo {
        TaskInfo {
            thread: task.started.thread,
            project: task.project.project,
            kind: TaskKind::Agent,
            title: task.started.title.clone(),
            model: task.started.model.clone(),
            tokens: task.tokens,
            cost: task.cost,
            budget: task.started.budget,
            started_at: task.at,
            elapsed_ms: self.elapsed(task.at, None),
            status: TaskStatus::Running,
            outcome: None,
            schedule: task.started.schedule.map(|fired| fired.schedule),
        }
    }

    /// `task` of `project`'s ledger, as lists show it.
    fn recorded_info(&self, project: Located, task: &Recorded, status: TaskStatus) -> TaskInfo {
        let ended = task.ended.as_ref();
        TaskInfo {
            thread: task.started.thread,
            project: project.project,
            kind: TaskKind::Agent,
            title: task.started.title.clone(),
            model: task.started.model.clone(),
            tokens: ended.map(|(ended, _)| ended.tokens).unwrap_or_default(),
            cost: ended.and_then(|(ended, _)| ended.cost),
            budget: task.started.budget,
            started_at: task.at,
            elapsed_ms: self.elapsed(task.at, ended.map(|(_, at)| *at)),
            status,
            outcome: ended.map(|(ended, _)| ended.outcome),
            schedule: task.started.schedule.map(|fired| fired.schedule),
        }
    }

    fn elapsed(&self, from: At, to: Option<At>) -> u64 {
        let to = to.map_or_else(|| self.inner.harness.clock().now(), At::timestamp);
        let millis = to
            .as_millisecond()
            .saturating_sub(from.timestamp().as_millisecond());
        u64::try_from(millis).unwrap_or(0)
    }
}

/// How `thread`'s turn ended, when `note` is its `turn.ended`.
fn turn_ended(thread: ThreadId, note: &Notification) -> Option<TurnResult> {
    let Notification::Event {
        thread: from,
        event,
    } = note
    else {
        return None;
    };
    if *from != thread || event.kind.as_str() != TurnResult::TYPE {
        return None;
    }
    match decode(event.clone()).ok()?.body {
        Body::TurnEnded(result) => Some(result),
        _ => None,
    }
}

/// How a task ended: the turn's own totals when it reports them, else the tally. One the
/// server's stop cut short, not the person, was interrupted.
fn settle(running: &Running, ended: &Result<TurnResult, ApiError>, stopping: bool) -> TaskEnded {
    let (outcome, tokens, cost) = match ended {
        Ok(turn) => (turn.outcome, turn.tokens, turn.cost),
        Err(error) if error.code == code::CANCELLED => {
            let outcome = if stopping && !running.stopped {
                Outcome::Interrupted
            } else {
                Outcome::Cancelled
            };
            (outcome, running.tokens, running.cost)
        }
        // Stopped while followed by its thread's records, before its turn's end came.
        Err(_) if running.stopped => (Outcome::Cancelled, running.tokens, running.cost),
        Err(_) => (Outcome::Failed, running.tokens, running.cost),
    };
    let outcome = if running.over_budget && !running.stopped {
        Outcome::LimitReached
    } else {
        outcome
    };
    TaskEnded {
        thread: running.started.thread,
        due: running.started.schedule.map(|fired| fired.due),
        outcome,
        tokens,
        cost,
        by: running.stopped.then(|| {
            running
                .stopped_by
                .map_or_else(Principal::glave, Principal::person)
        }),
    }
}

impl<P, L, B> Server<P, L, B> {
    /// Stops following every task followed by its thread's records, as the server stops: each
    /// stays open in the ledger, for the next server to adopt.
    pub(crate) fn unfollow_tasks(&self) {
        let tasks = self
            .inner
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        #[expect(
            clippy::iter_over_hash_type,
            reason = "each is told alike; the order is never seen"
        )]
        for task in tasks.values() {
            task.unfollowed.interrupt();
        }
    }

    /// Keeps `task` with the server's others, so shutting down waits for it.
    pub(crate) fn own(&self, task: tokio::task::JoinHandle<()>) {
        let mut tasks = self
            .inner
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }
}
