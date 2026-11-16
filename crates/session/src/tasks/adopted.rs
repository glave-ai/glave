//! A schedule's run that waits for the person when its server stops: the server that runs tasks
//! next adopts it as it starts, rather than recording it interrupted, so its budget holds when
//! the person answers and it goes on, and its end is recorded as any task's is.

use glave_event::{
    At, Body, Currency, Money, Nanos, Payload as _, ThreadId, TokenCounts, TurnResult, TurnStarted,
    decode,
};
use glave_ports::{BlobStore, Caller, EventLog, Located, ModelProvider, Scope};
use tokio::sync::{broadcast, watch};

use super::Running;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::turn::TurnStream;
use crate::words::Sentence;

/// The most model calls a run's spending so far is counted from.
const MOST_CALLS: usize = 10_000;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Adopts the task open in `project`'s ledger for `thread` when it is a schedule's run whose
    /// turn waits for the person: followed here from now on, with what it spent so far counted
    /// against its budget. Says whether it did.
    pub(super) async fn adopt_waiting(
        &self,
        project: Located,
        thread: ThreadId,
    ) -> Result<bool, ApiError> {
        let open =
            (self.ledgers().get(&project)).and_then(|ledger| ledger.open.get(&thread).cloned());
        let Some(recorded) = open.filter(|task| task.started.schedule.is_some()) else {
            return Ok(false);
        };
        // As whoever's run it is.
        let caller = Caller {
            person: super::whose(recorded.started.by.as_ref()),
            org: Some(project.org),
            ..Caller::default()
        };
        let scope = self.authorize(&caller, thread).await?;
        let view = self.inner.harness.log().thread_view(&scope, thread).await?;
        let Some(view) = view else {
            return Ok(false);
        };
        if !self.view_waits(&scope, &view).await? {
            return Ok(false);
        }
        let (tokens, cost) = self.spent_since(&scope, thread, recorded.at).await?;
        let watching = self.watch_from_now(&caller, scope, thread).await?;
        // Its end comes from its turn's own `turn.ended`, whoever goes on with it.
        let (finish, done) = watch::channel(None);
        let mut stream =
            TurnStream::new(recorded.started.model.clone(), thread, Some(watching), done);
        stream.outlasting_access();
        let running = Running {
            project,
            started: recorded.started,
            at: recorded.at,
            by: caller.person,
            tokens,
            cost,
            stopped: false,
            stopped_by: None,
            over_budget: false,
        };
        self.tasks().insert(thread, running);
        // Whoever goes on with the turn sends its end here.
        self.inner.followed.keep(thread, finish);
        let server = self.clone();
        let follow = tokio::spawn(async move { server.follow_task(thread, stream).await });
        self.own(follow);
        Ok(true)
    }

    /// How `thread`'s turn ends, followed by what its thread records from now on, its model
    /// calls counted against its task's budget: for a turn whose follower was let go while it
    /// waited, as when more turns wait than are kept followed.
    pub(super) async fn follow_by_notes(&self, thread: ThreadId) -> Result<TurnResult, ApiError> {
        let (mut notes, _) = self.inner.hub.watch(thread);
        // It may have ended before this listened.
        if let Some(result) = self.ended_already(thread).await? {
            return Ok(result);
        }
        loop {
            match notes.recv().await {
                Ok(note) => {
                    self.count(thread, &note);
                    if let Some(result) = super::turn_ended(thread, &note) {
                        return Ok(result);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if let Some(result) = self.ended_already(thread).await? {
                        return Ok(result);
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(ApiError::new(code::INTERNAL, Sentence::TaskStopped));
                }
            }
        }
    }

    /// How `thread`'s latest turn ended, when it has.
    async fn ended_already(&self, thread: ThreadId) -> Result<Option<TurnResult>, ApiError> {
        let scope = self.scope_of(thread)?;
        let kinds = [TurnStarted::TYPE, TurnResult::TYPE];
        let log = self.inner.harness.log();
        let latest = log.read_back(&scope, thread, Some(&kinds), None, 1).await?;
        Ok(latest
            .into_iter()
            .next()
            .and_then(|raw| match decode(raw).ok()?.body {
                Body::TurnEnded(result) => Some(result),
                _ => None,
            }))
    }

    /// The tokens `thread`'s model calls used since `since`, and what they cost when every call
    /// said.
    async fn spent_since(
        &self,
        scope: &Scope,
        thread: ThreadId,
        since: At,
    ) -> Result<(TokenCounts, Option<Money>), ApiError> {
        let kinds = ["model.responded"];
        let log = self.inner.harness.log();
        let calls = log
            .read_back(scope, thread, Some(&kinds), None, MOST_CALLS)
            .await?;
        let mut tokens = TokenCounts::default();
        let mut cost = Some(Money {
            nanos: Nanos::ZERO,
            currency: Currency::USD,
        });
        for raw in calls.into_iter().take_while(|raw| raw.at >= since) {
            if let Ok(Body::ModelResponded(responded)) = decode(raw).map(|event| event.body) {
                tokens += responded.usage.counts();
                cost = cost
                    .zip(responded.cost)
                    .and_then(|(sum, cost)| sum.checked_add(cost.amount));
            }
        }
        Ok((tokens, cost))
    }
}
