//! Goals over the session API: `goal/start` works toward a goal across turns and streams them
//! as `turn/start` streams one, `goal/stop` stops it, and `goal/get` reads its progress from
//! the log, so any client sees it while it runs.

use std::sync::Arc;

use glave_event::ThreadId;
use glave_harness::{Goal, GoalProgress};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::held::{Slot, Use, finished};
use crate::protocol::{ApiError, Reply, code};
use crate::server::Server;
use crate::turn::{TurnStream, publish};

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Sets `goal` on `thread` for `caller` and streams its turns: their notifications, then
    /// how the last one ended, with every turn's calls, tokens and cost added up. Without a
    /// goal, continues the thread's active one or resumes the one the model paused.
    pub async fn start_goal_as(
        &self,
        caller: &Caller,
        thread: ThreadId,
        goal: Option<Goal>,
    ) -> Result<TurnStream, ApiError> {
        self.authorize(caller, thread).await?;
        // A model the thread chose before must still be one this server runs.
        let model = self.model_of(thread, None).await?;
        // So must the approval mode it chose before be one the policy leaves.
        self.check_approval(thread, None).await?;
        if goal.is_none() && !self.goal(thread).await?.is_some_and(|g| g.resumable()) {
            return Err(no_goal(thread));
        }
        let (finish, done) = watch::channel(None);
        let notes = self.watch(thread);
        let caller = caller.clone();
        let work = move |inner: Arc<crate::server::Inner<P, L, B>>, mut held, cancel| async move {
            let hub = Arc::clone(&inner.hub);
            let mut live = |live| publish(&hub, thread, live);
            let harness = &inner.harness;
            let ran = match goal {
                Some(goal) => {
                    harness
                        .start_goal(&mut held, goal, &caller, &cancel, &mut live)
                        .await
                }
                None => {
                    harness
                        .continue_goal(&mut held, &caller, &cancel, &mut live)
                        .await
                }
            };
            // What was sent after the goal's last call starts a turn of its own.
            let ran = harness.run_queued(&mut held, ran, &cancel, &mut live).await;
            (held, ran)
        };
        self.occupy_for(thread, finish, Use::Steered, work).await?;
        Ok(TurnStream::new(model, Some(notes), done))
    }

    /// Stops `thread`'s active goal for `caller`: cancels its running turn, if any, and
    /// records the goal as stopped.
    pub(crate) async fn stop_goal(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<Reply, ApiError> {
        if !self.goal(thread).await?.is_some_and(|g| g.active()) {
            return Err(no_goal(thread));
        }
        let running = match self.lock().get(&thread) {
            Some(Slot::Busy {
                cancel, finished, ..
            }) => {
                cancel.cancel();
                Some(finished.clone())
            }
            _ => None,
        };
        if let Some(mut running) = running {
            let _ = running.wait_for(|done| *done).await;
        }
        // A goal's cancelled turn has stopped it already; a plain turn's has not.
        let (finish, mut done) = watch::channel(None);
        let caller = caller.clone();
        self.occupy(thread, finish, move |inner, mut held, _| async move {
            let stopped = inner.harness.stop_goal(&mut held, &caller).await;
            (held, stopped)
        })
        .await?;
        let ended = finished(&mut done)
            .await
            .ok_or_else(|| ApiError::new(code::INTERNAL, "stopping the goal stopped"))?;
        ended.as_ref().as_ref().map_err(ApiError::from)?;
        Ok(Reply::Done {})
    }

    /// `thread`'s latest goal, read from the log.
    pub(crate) async fn goal(&self, thread: ThreadId) -> Result<Option<GoalProgress>, ApiError> {
        let scope = self.scope_of(thread).await?;
        self.inner
            .harness
            .goal(scope, thread)
            .await
            .map_err(|error| ApiError::from(&error))
    }
}

fn no_goal(thread: ThreadId) -> ApiError {
    ApiError::new(code::NO_GOAL, format!("thread {thread} has no active goal"))
}
