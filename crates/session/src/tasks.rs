//! Tasks: a turn, or a goal's turns, run in a thread with no one attached. The server starts
//! the work as it would for a client, answers its approvals "no one there" (in auto mode the
//! judge still clears what the person asked for), keeps a running tally of its tokens and cost
//! from the `model.responded` events it records, stops a turn at its budget, and records the
//! task's start and end in the space's ledger. `tasks/list` and `tasks/stop` show and stop
//! them; `task/finished` tells the thread's watchers when one ends.

use std::collections::HashMap;

use glave_event::{
    At, Body, Currency, Fired, Money, Outcome, Principal, PrincipalId, TaskEnded, TaskStarted,
    ThreadId, TokenCounts, decode,
};
use glave_harness::{Caller, Goal};
use glave_ports::{BlobStore, EventLog, ModelProvider, title_from};

use crate::held::{Slot, busy};
use crate::ledger::Recorded;
use crate::modes::Choice;
use crate::protocol::{
    ApiError, Notification, Reply, TaskInfo, TaskKind, TaskStatus, TurnResult, code,
};
use crate::server::Server;
use crate::steer::{Message, Sent};
use crate::turn::{TurnItem, TurnStream};

/// What a person may run at once and keep scheduled (`[tasks]` in `policy.toml`): no limit
/// when `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskLimits {
    pub max_running: Option<usize>,
    pub max_schedules: Option<usize>,
}

/// What a task does.
pub(crate) enum Work {
    /// One turn with this prompt, with the model and approval mode chosen with it, if any.
    Turn(String, Choice),
    /// A goal's turns: a new goal, or the thread's active or paused one.
    Goal(Option<Goal>),
}

/// A task running in this server.
#[derive(Debug, Clone)]
pub(crate) struct Running {
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

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Refuses a task in `thread` for `caller` outside this server's own space, beyond the
    /// policy's limit, or while a turn runs there.
    async fn may_start_task(&self, caller: &Caller, thread: ThreadId) -> Result<(), ApiError> {
        // A space's tasks are kept in its own ledger; this server keeps only its own space's.
        if self.authorize(caller, thread).await?.space != self.inner.home.space {
            return Err(ApiError::new(
                code::INVALID_PARAMS,
                "tasks run only in threads of this server's own space",
            ));
        }
        if let Some(max) = self.inner.limits.max_running {
            let running = self.running_for(caller.person);
            if running >= max {
                return Err(ApiError::new(
                    code::LIMIT,
                    format!(
                        "{running} tasks are running, the most policy allows ([tasks] \
                         max_running = {max}); stop one first"
                    ),
                ));
            }
        }
        // Checked before no one answers for the thread, so a turn someone follows keeps its
        // answers.
        let busy = matches!(self.lock().get(&thread), Some(Slot::Busy { .. }));
        if busy || self.tasks().contains_key(&thread) {
            return Err(ApiError::new(
                code::TURN_RUNNING,
                format!("a turn is already running in thread {thread}"),
            ));
        }
        Ok(())
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
        self.may_start_task(caller, thread).await?;
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
                    prompt: &prompt,
                    choice,
                    queue: false,
                };
                self.start_turn_with(caller, thread, message, None).await
            }
            .and_then(|sent| match sent {
                Sent::Started(stream) => Ok(stream),
                Sent::Queued(_) => Err(busy(thread)),
            }),
            Work::Goal(goal) => self.start_goal_as(caller, thread, goal).await,
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
        if let Err(error) = self.record_in_ledger(&started).await {
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
                None => break Err(ApiError::new(code::INTERNAL, "the task stopped")),
            }
        };
        self.inner.approvals.attend(thread);
        let Some(running) = self.tasks().remove(&thread) else {
            return;
        };
        let ended = settle(&running, &ended);
        // Recorded on a best effort: the thread's own events hold the work either way.
        let _ = self.record_in_ledger(&ended).await;
        let now = At::from(self.inner.harness.clock().now());
        let recorded = Recorded {
            started: running.started,
            at: running.at,
            ended: Some((ended, now)),
        };
        let task = self.recorded_info(&recorded, TaskStatus::Finished);
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
            add(&mut running.tokens, &responded.usage.counts());
            running.cost = match (running.cost, &responded.cost) {
                (Some(sum), Some(cost)) if sum.currency == cost.amount.currency => Some(Money {
                    micros: sum.micros.saturating_add(cost.amount.micros),
                    currency: sum.currency,
                }),
                _ => None,
            };
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
                    format!("no task is running in thread {thread} in this server"),
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

    /// Tasks running here and recently finished, running first, then newest first.
    pub(crate) async fn list_tasks(&self, limit: Option<usize>) -> Result<Reply, ApiError> {
        self.catch_up().await?;
        let limit = limit.unwrap_or(20).clamp(1, 500);
        let mut running: Vec<TaskInfo> = self
            .tasks()
            .values()
            .map(|task| self.running_info(task))
            .collect();
        running.sort_by_key(|task| std::cmp::Reverse(task.started_at));
        let (mut elsewhere, finished) = {
            let ledger = self.ledger();
            let here = self.tasks();
            let elsewhere: Vec<TaskInfo> = ledger
                .open
                .values()
                .filter(|task| !here.contains_key(&task.started.thread))
                .map(|task| self.recorded_info(task, TaskStatus::Elsewhere))
                .collect();
            let finished: Vec<TaskInfo> = ledger
                .finished
                .iter()
                .rev()
                .map(|task| self.recorded_info(task, TaskStatus::Finished))
                .collect();
            (elsewhere, finished)
        };
        elsewhere.sort_by_key(|task| std::cmp::Reverse(task.started_at));
        running.extend(elsewhere);
        running.extend(finished);
        running.truncate(limit);
        Ok(Reply::Tasks { tasks: running })
    }

    /// Records every task the ledger has open and this server does not run as interrupted:
    /// for the one server that runs a space's tasks, when it starts.
    pub async fn recover_tasks(&self) -> Result<usize, ApiError> {
        self.catch_up().await?;
        let stale: Vec<ThreadId> = {
            let here = self.tasks();
            self.ledger()
                .open
                .keys()
                .filter(|thread| !here.contains_key(thread))
                .copied()
                .collect()
        };
        for thread in &stale {
            let ended = TaskEnded {
                thread: *thread,
                outcome: Outcome::Interrupted,
                tokens: TokenCounts::default(),
                cost: None,
                by: None,
            };
            self.record_in_ledger(&ended).await?;
        }
        Ok(stale.len())
    }

    /// How many tasks run in this server now.
    pub fn running_tasks(&self) -> usize {
        self.tasks().len()
    }

    pub(crate) fn tasks(&self) -> std::sync::MutexGuard<'_, Tasks> {
        self.inner
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `person` as a task records them: absent for the space's own person.
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

    pub(crate) fn recorded_info(&self, task: &Recorded, status: TaskStatus) -> TaskInfo {
        let ended = task.ended.as_ref();
        TaskInfo {
            thread: task.started.thread,
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

/// How a task ended: the turn's own totals when it reports them, else the tally.
fn settle(running: &Running, ended: &Result<TurnResult, ApiError>) -> TaskEnded {
    let (outcome, tokens, cost) = match ended {
        Ok(turn) => (turn.outcome, turn.tokens, turn.cost),
        Err(error) if error.code == code::CANCELLED => {
            (Outcome::Cancelled, running.tokens, running.cost)
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

fn add(total: &mut TokenCounts, counts: &TokenCounts) {
    total.input = total.input.saturating_add(counts.input);
    total.cache_read = total.cache_read.saturating_add(counts.cache_read);
    total.cache_write = total.cache_write.saturating_add(counts.cache_write);
    total.output = total.output.saturating_add(counts.output);
    total.reasoning = total.reasoning.saturating_add(counts.reasoning);
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
