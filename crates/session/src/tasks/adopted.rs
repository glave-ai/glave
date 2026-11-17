//! A schedule's run that waits for the person when its server stops: the server that runs tasks
//! next adopts it as it starts, under the task's lease, rather than recording it interrupted, so
//! its budget holds when the person answers and it goes on, and its end is recorded as any
//! task's is, once. It is followed by what its thread records: what this server's hub hears of
//! it, and its turn's end looked for in the log, since another server may go on with it. One
//! whose maker was removed from its organization, or whose people can't be read to tell, is not
//! adopted, so it is recorded interrupted.

use std::time::Duration;

use glave_event::{
    At, Body, Currency, Money, Nanos, Payload as _, Seq, ThreadId, TokenCounts, ToolCallRequested,
    TurnResult, TurnStarted, decode,
};
use glave_harness::Cancel;
use glave_ports::{BlobStore, CallKind, EventLog, Lease, Located, ModelProvider, Scope};
use tokio::sync::broadcast;

use super::Running;
use crate::protocol::{ApiError, code};
use crate::server::Server;
use crate::words::Sentence;

/// The most model calls, or calls for subagents, a run's spending so far is counted from.
const MOST_CALLS: usize = 10_000;

/// How often a run followed by its thread's records looks in the log for its turn's end: one
/// another server went on with is never heard of here.
const LOOK: Duration = Duration::from_secs(30);

/// Turn starts and ends read back to find the end of the turn a run follows.
const MOST_TURNS: usize = 256;

/// Tokens a run spent so far, and what they cost while every call said.
type Spent = (TokenCounts, Option<Money>);

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Adopts the task open in `project`'s ledger for `thread` when it is a schedule's run whose
    /// turn waits for the person: followed here from now on, under `lease`, the task's, with what
    /// it spent so far counted against its budget. Says whether it did; the lease is the
    /// follower's then, and the caller's to let go of otherwise.
    pub(super) async fn adopt_waiting(
        &self,
        project: Located,
        thread: ThreadId,
        lease: Lease,
    ) -> Result<bool, ApiError> {
        let open =
            (self.ledgers().get(&project)).and_then(|ledger| ledger.open.get(&thread).cloned());
        let Some(recorded) = open.filter(|task| task.started.schedule.is_some()) else {
            return Ok(false);
        };
        // As whoever's run it is: never one of someone glave can't tell, or of anyone its
        // organization's people ledger doesn't list as there now, or can't be read to tell.
        let Ok(person) = super::whose(recorded.started.by.as_ref()) else {
            return Ok(false);
        };
        let caller = self.acting_as(person, project.org);
        match self.membership(&caller).await {
            Ok(super::Membership::Member) => {}
            Ok(super::Membership::Removed | super::Membership::Unknown) => return Ok(false),
            Err(error) => {
                tracing::warn!(%error, org = %project.org, "could not read the organization's people");
                return Ok(false);
            }
        }
        let scope = self.authorize(&caller, thread).await?;
        let view = self.inner.harness.log().thread_view(&scope, thread).await?;
        let Some(view) = view else {
            return Ok(false);
        };
        if !self.view_waits(&scope, &view).await? {
            return Ok(false);
        }
        let (tokens, cost) = self.spent_since(&scope, thread, recorded.at).await?;
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
            unfollowed: Cancel::new(),
        };
        let unfollowed = running.unfollowed.clone();
        self.tasks().insert(thread, running);
        let server = self.clone();
        self.follow_held(lease, unfollowed, async move {
            let ended = server.follow_by_notes(thread).await;
            server.task_ended(thread, ended).await;
        });
        Ok(true)
    }

    /// How `thread`'s turn, the one open as this starts, ends: told by what this server's hub
    /// hears of it, its model calls counted against its task's budget, and looked for in the log
    /// every [`LOOK`], since another server may go on with it. Refused as not followed once its
    /// task or the server stops following it.
    pub(super) async fn follow_by_notes(&self, thread: ThreadId) -> Result<TurnResult, ApiError> {
        let (mut notes, _) = self.inner.hub.watch(thread);
        let unfollowed = (self.tasks().get(&thread)).map(|task| task.unfollowed.clone());
        let unfollowed = unfollowed.unwrap_or_default();
        let scope = self.scope_of(thread)?;
        let view = self.inner.harness.log().thread_view(&scope, thread).await?;
        let started = view.and_then(|view| view.turn).map(|turn| turn.started);
        // Its first tick is at once: the turn may have ended before this listened.
        let mut look = tokio::time::interval(LOOK);
        loop {
            tokio::select! {
                biased;
                note = notes.recv() => match note {
                    Ok(note) => {
                        self.count(thread, &note);
                        if let Some(result) = super::turn_ended(thread, &note) {
                            return Ok(result);
                        }
                    }
                    // What was missed is in the log, which the next look reads.
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(ApiError::new(code::INTERNAL, Sentence::TaskStopped));
                    }
                },
                () = unfollowed.cancelled() => {
                    return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::TurnUnfollowed));
                }
                _ = look.tick() => {
                    if let Some(result) = self.ended_after(&scope, thread, started).await? {
                        return Ok(result);
                    }
                }
            }
        }
    }

    /// How the turn of `thread` begun at `started` ended, once it has: the first start or end
    /// recorded after it, when that is an end. With no start, its latest turn's end.
    async fn ended_after(
        &self,
        scope: &Scope,
        thread: ThreadId,
        started: Option<Seq>,
    ) -> Result<Option<TurnResult>, ApiError> {
        let kinds = [TurnStarted::TYPE, TurnResult::TYPE];
        let log = self.inner.harness.log();
        let latest = log
            .read_back(scope, thread, Some(&kinds), None, MOST_TURNS)
            .await?;
        // Newest first.
        let next = match started {
            Some(started) => latest
                .into_iter()
                .take_while(|raw| raw.seq > started)
                .last(),
            None => latest.into_iter().next(),
        };
        Ok(next.and_then(|raw| match decode(raw).ok()?.body {
            Body::TurnEnded(result) => Some(result),
            _ => None,
        }))
    }

    /// The tokens `thread`'s run spent since `since`, its subagents' included, and what they
    /// cost when every call said.
    async fn spent_since(
        &self,
        scope: &Scope,
        thread: ThreadId,
        since: At,
    ) -> Result<Spent, ApiError> {
        let mut spent = (
            TokenCounts::default(),
            Some(Money {
                nanos: Nanos::ZERO,
                currency: Currency::USD,
            }),
        );
        self.add_spent(scope, thread, Some(since), &mut spent)
            .await?;
        // Each subagent it asked for since then: its thread is that call's alone.
        let kinds = [ToolCallRequested::TYPE];
        let log = self.inner.harness.log();
        let asked = log
            .read_back(scope, thread, Some(&kinds), None, MOST_CALLS)
            .await?;
        for raw in asked.into_iter().take_while(|raw| raw.at >= since) {
            let Ok(Body::ToolCallRequested(call)) = decode(raw).map(|event| event.body) else {
                continue;
            };
            if CallKind::of(&call.tool) != CallKind::Agent {
                continue;
            }
            if let Some(child) = self.child_view(scope, thread, &call.call_id).await? {
                self.add_spent(scope, child.thread, None, &mut spent)
                    .await?;
            }
        }
        Ok(spent)
    }

    /// Adds what `thread`'s model calls since `since`, or all of them, used to `spent`.
    async fn add_spent(
        &self,
        scope: &Scope,
        thread: ThreadId,
        since: Option<At>,
        spent: &mut Spent,
    ) -> Result<(), ApiError> {
        let kinds = ["model.responded"];
        let log = self.inner.harness.log();
        let calls = log
            .read_back(scope, thread, Some(&kinds), None, MOST_CALLS)
            .await?;
        let counted = calls
            .into_iter()
            .take_while(|raw| since.is_none_or(|since| raw.at >= since));
        for raw in counted {
            if let Ok(Body::ModelResponded(responded)) = decode(raw).map(|event| event.body) {
                spent.0 += responded.usage.counts();
                spent.1 = (spent.1)
                    .zip(responded.cost)
                    .and_then(|(sum, cost)| sum.checked_add(cost.amount));
            }
        }
        Ok(())
    }
}
