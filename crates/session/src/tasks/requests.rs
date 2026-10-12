//! The requests about tasks and schedules: `turn/start` and `goal/start` with `background`,
//! `tasks/*` and `schedule/*`.

use glave_harness::{Caller, DEFAULT_BUDGET, Goal};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::modes::Choice;
use crate::protocol::{ApiError, Reply, Request, code};
use crate::schedule::NewSchedule;
use crate::server::Server;
use crate::tasks::Work;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers a request about tasks or schedules; hands any other back.
    pub(crate) async fn tasks_request(
        &self,
        caller: &Caller,
        request: Request,
    ) -> Result<Result<Reply, ApiError>, Box<Request>> {
        Ok(match request {
            Request::TurnStart {
                thread,
                prompt,
                background: true,
                budget,
                model,
                approval,
                ..
            } => {
                let work = Work::Turn(prompt, Choice { model, approval });
                self.start_task(caller, thread, work, budget, None)
                    .await
                    .map(|task| Reply::Task { task })
            }
            Request::GoalStart {
                thread,
                objective,
                budget,
                background: true,
            } => {
                let goal = objective.map(|objective| Goal {
                    objective,
                    token_budget: budget.unwrap_or(DEFAULT_BUDGET),
                });
                let budget = goal.as_ref().map(|goal| goal.token_budget);
                self.start_task(caller, thread, Work::Goal(goal), budget, None)
                    .await
                    .map(|task| Reply::Task { task })
            }
            Request::TasksList { limit } => self.list_tasks(limit).await,
            Request::TasksStop { thread } => self.stop_task(caller, thread).await,
            Request::ScheduleCreate {
                when,
                prompt,
                goal,
                budget,
                worktree,
            } => {
                let new = NewSchedule {
                    when,
                    prompt,
                    goal,
                    budget,
                    worktree,
                };
                self.create_schedule(caller, new).await
            }
            Request::ScheduleList {} => self.list_schedules().await,
            Request::ScheduleDelete { schedule } => self.delete_schedule(caller, schedule).await,
            other => return Err(Box::new(other)),
        })
    }
}

/// A turn's budget is enforced only for a turn that runs as a task: refused anywhere else
/// rather than ignored.
pub(crate) fn unenforced_budget() -> ApiError {
    ApiError::new(
        code::INVALID_PARAMS,
        "turn/start takes a budget only with background: true; for a turn you follow, use a goal",
    )
}
