//! A thread's plan over the session API (`specs/session.md`, "Plans"): `thread/plan` reads it
//! from the log, `plan/edit` records the person's steps in place of a proposal's, and
//! `plan/build` approves it and streams the turn that builds it, as `turn/start` streams one.
//! Changing it with words is a message (`turn/start`) in plan mode.

use std::sync::Arc;

use glave_event::{PlanStatus, ThreadId};
use glave_harness::{HarnessError, MAX_STEP, MAX_STEPS, Plan};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use tokio::sync::watch;

use crate::held::{Use, finished};
use crate::protocol::{ApiError, PlanInfo, Reply, code};
use crate::server::Server;
use crate::turn::{TurnStream, publish};
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// `thread`'s plan, read from the log.
    pub(crate) async fn plan(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let scope = self.scope_of(thread).await?;
        let plan = self.inner.harness.plan(scope, thread).await;
        let plan = plan.map_err(|error| ApiError::from(&error))?;
        Ok(Reply::Plan {
            plan: plan.map(info),
        })
    }

    /// Replaces the steps of `thread`'s proposed plan with `caller`'s, and answers with the
    /// plan. Steps a plan cannot hold are refused with a sentence the person can act on.
    pub(crate) async fn edit_plan(
        &self,
        caller: &Caller,
        thread: ThreadId,
        steps: Vec<String>,
    ) -> Result<Reply, ApiError> {
        if !holds(&steps) {
            let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
            let refused = Sentence::PlanStepsInvalid {
                most: count(MAX_STEPS),
                longest: count(MAX_STEP),
            };
            return Err(ApiError::new(code::INVALID_PARAMS, refused));
        }
        let (finish, mut done) = watch::channel(None);
        let by = caller.clone();
        self.occupy_for(
            thread,
            finish,
            Use::Work,
            move |inner, mut held, _| async move {
                let edited = inner.harness.edit_plan(&mut held, &by, &steps).await;
                (held, edited)
            },
        )
        .await?;
        let ended = finished(&mut done)
            .await
            .ok_or_else(|| ApiError::new(code::INTERNAL, Sentence::ChangeStopped))?;
        ended.as_ref().as_ref().map_err(ApiError::from)?;
        self.plan(thread).await
    }

    /// Builds `thread`'s proposed plan for `caller` and streams the turn that builds it: its
    /// notifications, then how it ended. Refused when no plan waits for approval (`-32033`),
    /// and where the policy leaves only Plan (`-32012`).
    pub(crate) async fn build_plan(
        &self,
        caller: &Caller,
        thread: ThreadId,
    ) -> Result<TurnStream, ApiError> {
        let scope = self.authorize(caller, thread).await?;
        // A model the thread chose before must still be one this server runs.
        let model = self.model_of(thread, None).await?;
        if self.inner.harness.builds_in().is_none() {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::NothingBuilt));
        }
        let proposed = self.inner.harness.plan(scope, thread).await;
        let proposed = proposed.map_err(|error| ApiError::from(&error))?;
        if !proposed.is_some_and(|plan| plan.proposed) {
            return Err(ApiError::from(&HarnessError::NoPlan(thread)));
        }
        let (finish, done) = watch::channel(None);
        let watch = self.watch_from_now(scope, thread).await?;
        let caller = caller.clone();
        let followed = finish.clone();
        let work = move |inner: Arc<crate::server::Inner<P, L, B>>, mut held, cancel| async move {
            let hub = Arc::clone(&inner.hub);
            let mut live = |live| publish(&hub, thread, live);
            let harness = &inner.harness;
            let ran = harness
                .build_plan(&mut held, &caller, &cancel, &mut live)
                .await;
            // What was sent while it ran starts a turn of its own.
            let ran = harness.run_queued(&mut held, ran, &cancel, &mut live).await;
            // A turn that stops at a wait goes on once answered.
            if matches!(ran, Err(HarnessError::Waits(_))) {
                inner.followed.keep(thread, followed);
            }
            (held, ran)
        };
        self.occupy_for(thread, finish, Use::Steered, work).await?;
        Ok(TurnStream::new(model, thread, Some(watch), done))
    }
}

/// Whether a plan holds `steps`: 1 to [`MAX_STEPS`] of them, none blank and none longer than
/// [`MAX_STEP`] characters.
fn holds(steps: &[String]) -> bool {
    let fits = |step: &String| {
        let step = step.trim();
        !step.is_empty() && step.chars().count() <= MAX_STEP
    };
    (1..=MAX_STEPS).contains(&steps.len()) && steps.iter().all(fits)
}

/// `plan` as the API gives it, with its sentence: proposed, or how far it has come.
fn info(plan: Plan) -> PlanInfo {
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let steps = count(plan.steps.len());
    let sentence = if plan.proposed {
        Sentence::PlanProposed { steps }
    } else {
        let done = plan
            .steps
            .iter()
            .filter(|step| step.status == PlanStatus::Completed)
            .count();
        Sentence::PlanProgress {
            done: count(done),
            steps,
        }
    };
    PlanInfo {
        steps: plan.steps,
        proposed: plan.proposed,
        sentence: sentence.into(),
    }
}
