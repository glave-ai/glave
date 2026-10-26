//! Tasks: a turn, or a goal's turns, run in a thread with no one attached, in any project its
//! caller works in, always on a copy of the project's folder (a worktree, a worker or a scratch
//! folder), never in the folder itself. The server starts the work as it would for a client,
//! answers its approvals "no one there" (in auto mode the judge still clears what the person
//! asked for), keeps a running tally of its tokens and cost from the `model.responded` events
//! it records, stops a turn at its budget, and records the task's start and end in its project's
//! ledger. `tasks/list` and `tasks/stop` show and stop them; `task/finished` tells the thread's
//! watchers when one ends.

use std::collections::HashMap;

use glave_event::{
    At, Body, Currency, Fired, Money, Opened, Outcome, Payload, Principal, PrincipalId, TaskEnded,
    TaskStarted, ThreadId, TokenCounts, WorkspaceBound, decode,
};
use glave_harness::Goal;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ProjectId, Scope, title_from};

use crate::choice::Choice;
use crate::held::{Slot, busy};
use crate::leased::Repaired;
use crate::ledger::Recorded;
use crate::protocol::{
    ApiError, Notification, Reply, TaskInfo, TaskKind, TaskStatus, TurnResult, code,
};
use crate::server::Server;
use crate::steer::{Message, Sent};
use crate::turn::{TurnItem, TurnStream};
use crate::words::Sentence;

/// What a person may run at once and keep scheduled (`[tasks]` in `policy.toml`): no limit
/// when `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskLimits {
    pub max_running: Option<usize>,
    pub max_schedules: Option<usize>,
}

/// What a task does.
pub(crate) enum Work {
    /// One turn with this prompt, with the model and mode chosen with it, if any.
    Turn(String, Choice),
    /// A goal's turns: a new goal, or the thread's active or paused one.
    Goal(Option<Goal>),
}

/// A task running in this server.
#[derive(Debug, Clone)]
pub(crate) struct Running {
    /// The project whose ledger records it.
    project: ProjectId,
    started: TaskStarted,
    at: At,
    goal: bool,
    by: Option<PrincipalId>,
    tokens: TokenCounts,
    cost: Option<Money>,
    /// Whether someone stopped it, and who, when the server knows.
    stopped: bool,
    stopped_by: Option<PrincipalId>,
    over_budget: bool,
}

pub(crate) type Tasks = HashMap<ThreadId, Running>;

/// Refused: a task asked for in the folder itself.
pub(crate) fn in_place() -> ApiError {
    ApiError::new(code::INVALID_PARAMS, Sentence::TaskInPlace)
}

/// Refused: no copy of the project's folder could be made for a task to work on.
pub(crate) fn no_copy() -> ApiError {
    ApiError::new(code::WORKSPACE, Sentence::TaskNoCopy)
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The scope of `thread`, refused for a task for `caller` where the thread works in its
    /// project's folder itself, beyond the policy's limit, or while a turn runs there.
    async fn may_start_task(&self, caller: &Caller, thread: ThreadId) -> Result<Scope, ApiError> {
        let scope = self.authorize(caller, thread).await?;
        if self.in_its_folder(&scope, thread).await? {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                Sentence::TaskThreadInFolder,
            ));
        }
        if let Some(max) = self.inner.limits.max_running {
            let running = self.running_for(caller.person);
            if running >= max {
                return Err(ApiError::new(
                    code::LIMIT,
                    Sentence::TooManyTasks { running, most: max },
                ));
            }
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
        schedule: Option<Fired>,
    ) -> Result<TaskInfo, ApiError> {
        let project = self.may_start_task(caller, thread).await?.project();
        let (title, goal) = match &work {
            Work::Turn(prompt, _) => (title_from(prompt).unwrap_or_default(), false),
            Work::Goal(Some(goal)) => (title_from(&goal.objective).unwrap_or_default(), true),
            Work::Goal(None) => ("continuing the thread's goal".to_owned(), true),
        };
        self.inner.approvals.unattend(thread);
        let stream = match work {
            // A task's own turn never queues: one running in the thread refuses it.
            Work::Turn(prompt, choice) => {
                let message = Message {
                    files: &[],
                    prompt: &prompt,
                    choice,
                    queue: false,
                    // Its thread is unattended for the whole task already.
                    unattended: false,
                };
                self.start_turn_with(caller, thread, message, None).await
            }
            .and_then(|sent| match sent {
                Sent::Started(stream) => Ok(stream),
                Sent::Queued(_) => Err(busy(thread)),
            }),
            Work::Goal(goal) => self.start_goal_as(caller, thread, goal, false).await,
        };
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                self.inner.approvals.attend(thread);
                return Err(error);
            }
        };
        let started = TaskStarted {
            thread,
            title,
            model: stream.model.clone(),
            budget,
            schedule,
            by: self.other_than_person(caller.person),
        };
        let running = Running {
            project,
            started: started.clone(),
            at: At::from(self.inner.harness.clock().now()),
            goal,
            by: caller.person,
            tokens: TokenCounts::default(),
            cost: Some(Money {
                micros: 0,
                currency: Currency::USD,
            }),
            stopped: false,
            stopped_by: None,
            over_budget: false,
        };
        let info = self.running_info(&running);
        self.tasks().insert(thread, running);
        if let Err(error) = self.record_in_ledger(project, &started).await {
            self.cancel_busy(thread);
            self.follow_task(thread, stream).await;
            return Err(error);
        }
        let server = self.clone();
        let follow = tokio::spawn(async move { server.follow_task(thread, stream).await });
        self.own(follow);
        Ok(info)
    }

    /// Follows a task's work to its end, then records how it ended and says so.
    async fn follow_task(&self, thread: ThreadId, mut stream: TurnStream) {
        let ended = loop {
            match stream.next().await {
                Some(TurnItem::Note(note)) => self.count(thread, &note),
                // A watcher this far behind is rare; the turn's own totals settle it at the end.
                Some(TurnItem::Missed(_)) => {}
                Some(TurnItem::Ended(ended)) => break ended,
                None => break Err(ApiError::new(code::INTERNAL, Sentence::TaskStopped)),
            }
        };
        self.inner.approvals.attend(thread);
        let Some(running) = self.tasks().remove(&thread) else {
            return;
        };
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
    /// turn that has reached its budget. A goal keeps to its budget itself.
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
            let over = !running.goal
                && !running.over_budget
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
        let goal = {
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
            running.goal
        };
        if goal {
            self.stop_goal(caller, thread).await?;
        } else if let Some(mut finished) = self.cancel_busy(thread) {
            let _ = finished.wait_for(|done| *done).await;
        }
        Ok(Reply::Done {})
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
        for project in self.task_projects().await? {
            self.catch_up(project).await?;
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
                let Some(lease) = self.take(thread).await? else {
                    recovered.held = recovered.held.saturating_add(1);
                    continue;
                };
                let ended = self.interrupt(project, thread).await;
                self.let_go(&lease).await;
                if ended? {
                    recovered.closed = recovered.closed.saturating_add(1);
                }
            }
        }
        Ok(recovered)
    }

    /// Records `thread`'s task in `project` interrupted, under its thread's lease, unless its
    /// ending was recorded meanwhile; says whether it recorded.
    async fn interrupt(&self, project: ProjectId, thread: ThreadId) -> Result<bool, ApiError> {
        self.catch_up(project).await?;
        let open = self
            .ledgers()
            .get(&project)
            .is_some_and(|ledger| ledger.open.contains_key(&thread));
        if !open {
            return Ok(false);
        }
        let ended = TaskEnded {
            thread,
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

    /// `person` as a task records them: absent for the project's own person.
    fn other_than_person(&self, person: Option<PrincipalId>) -> Option<Principal> {
        person
            .filter(|person| Some(*person) != self.inner.person)
            .map(Principal::person)
    }

    fn running_for(&self, person: Option<PrincipalId>) -> usize {
        self.tasks()
            .values()
            .filter(|task| task.by == person)
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
            project: task.project,
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
    fn recorded_info(&self, project: ProjectId, task: &Recorded, status: TaskStatus) -> TaskInfo {
        let ended = task.ended.as_ref();
        TaskInfo {
            thread: task.started.thread,
            project,
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
        Err(_) => (Outcome::Failed, running.tokens, running.cost),
    };
    let outcome = if running.over_budget && !running.stopped {
        Outcome::LimitReached
    } else {
        outcome
    };
    TaskEnded {
        thread: running.started.thread,
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
