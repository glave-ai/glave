//! A thread's plan over the session API (`specs/session.md`, "Plans"): `thread/plan` reads it
//! from the log.

use glave_event::{PlanStatus, ThreadId};
use glave_harness::Plan;
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::protocol::{ApiError, PlanInfo, Reply};
use crate::server::Server;
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
}

/// `plan` as the API gives it, with its sentence: how far it has come.
fn info(plan: Plan) -> PlanInfo {
    let count = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let done = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStatus::Completed)
        .count();
    let sentence = Sentence::PlanProgress {
        done: count(done),
        steps: count(plan.steps.len()),
    };
    PlanInfo {
        steps: plan.steps,
        sentence: sentence.into(),
    }
}
