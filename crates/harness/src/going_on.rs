//! A turn going on from its log once its wait was answered (`specs/one-server.md`, "An answer
//! continues the turn"): the answered call runs or is declined, the calls after it in that
//! response, which never started, run in order, and the turn's own loop goes on to the model.
//! It is the turn's one code path, entered at "the rest of this response's calls"; a goal's turn
//! goes on to the goal's next turns. Nothing of the turn is kept in memory meanwhile: what it did
//! so far is read from the log.

use glave_event::{Body, Event, Permissions, TurnResult};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider, read_all};
use serde::Deserialize;

use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::repeats::Repeats;
use crate::toolset::ToolSet;
use crate::turn::{CONFIG_LIMIT, Live, Step};
use crate::waits::{Opened, open_turn};

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Goes on with `thread`'s turn from its log, once what it waits for was answered, or once
    /// `cancel` fired, which declines what waits: as the turn would have gone on had it never
    /// stopped, for `caller`. A goal's turn goes on to the goal's next turns. A turn that waits
    /// still, or again, stops again (`HarnessError::Waits`); refused (`HarnessError::NoTurn`)
    /// when no turn is open there.
    pub async fn go_on(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        self.go_on_with(thread, None, caller, cancel, live).await
    }

    /// Like [`Harness::go_on`], with `tools` when they are not the thread's own, as a
    /// subagent's are its parent's. Boxed, so a subagent's turn can go on inside its parent's.
    pub(crate) fn go_on_with<'a>(
        &'a self,
        thread: &'a mut Thread,
        tools: Option<ToolSet>,
        caller: &'a Caller,
        cancel: &'a Cancel,
        live: &'a mut (dyn FnMut(Live) + Send),
    ) -> BoxFuture<'a, Result<TurnResult, HarnessError>> {
        Box::pin(async move {
            // What fails before the turn's steps leave it ended, or at a wait again, leaves it
            // as its log has it: still waiting, when nothing was answered; else open, which the
            // next work on the thread closes first, reading the log again.
            thread.opened = Some(Opened::LeftOpen);
            let events = self.load(&thread.scope, thread.id).await?;
            if self.opened(&thread.scope, thread.id, &events).await? == Some(Opened::Waits) {
                thread.opened = Some(Opened::Waits);
            }
            let open = open_turn(&events).ok_or(HarnessError::NoTurn(thread.id))?;
            // What was recorded since the thread was read, its answer among it, comes in.
            let last = events.last().ok_or(HarnessError::NotFound(thread.id))?;
            thread.next_seq = last
                .header
                .seq
                .next()
                .ok_or_else(|| HarnessError::Event(format!("thread {} is full", thread.id)))?;
            thread.messages = crate::conversation::conversation(&events);
            let turn = events.get(open.started..).unwrap_or_default();
            // It keeps the model and mode it started with, whatever the settings say now,
            // and its sandbox with the mode, so before the thread's own is brought.
            let (model, mode) = self.kept_by(thread, turn).await;
            let chosen = (thread.model.clone(), thread.permissions);
            thread.model = model.or_else(|| chosen.0.clone());
            thread.permissions = mode.or(chosen.1);
            let carried = async {
                self.reopen_removed(thread).await?;
                let mut toolset = if let Some(tools) = tools {
                    tools
                } else {
                    self.bring(thread).await?;
                    self.tools(thread).await
                };
                let goal = crate::goal::of_turn(thread, &events, open.started);
                let summary = self.tallies(&thread.scope, thread.id, turn).await?;
                if goal {
                    toolset.offer_goal();
                    crate::goal::going_on(thread, turn, &summary);
                }
                let step = Step {
                    toolset,
                    repeats: Repeats::default(),
                    summary,
                    answered: open.asked,
                    proposed: false,
                };
                let span = crate::telemetry::turn(thread.id, caller.trace.as_ref());
                let began = std::time::Instant::now();
                let ran = tracing::Instrument::instrument(
                    self.steps(thread, step, &open.calls, cancel, live),
                    span.clone(),
                )
                .await;
                let ended = ran.as_ref().ok();
                let ended = ended.map(|turn| (turn.outcome, turn.model_calls, turn.tool_calls));
                crate::telemetry::turn_ended(&span, began, ended);
                Ok::<_, HarnessError>((goal, ran))
            }
            .await;
            (thread.model, thread.permissions) = chosen;
            let (goal, ran) = carried?;
            if !goal {
                return ran;
            }
            crate::goal::stopped_running(thread);
            self.pursue_after(thread, ran?, caller, cancel, live).await
        })
    }

    /// The model the turn whose events are `turn` was sent to and the mode it ran with, as
    /// its `turn.started` and its configuration recorded them.
    async fn kept_by(
        &self,
        thread: &Thread,
        turn: &[Event],
    ) -> (Option<String>, Option<Permissions>) {
        let Some(Body::TurnStarted(started)) = turn.first().map(|event| &event.body) else {
            return (None, None);
        };
        let mode = async {
            let found = self.blobs.get(&thread.scope, &started.config.sha256).await;
            let mut blob = found.ok()??;
            let bytes = read_all(blob.bytes.as_mut(), CONFIG_LIMIT).await.ok()?;
            serde_json::from_slice::<Kept>(&bytes).ok()
        };
        let mode = mode.await.map(|kept| kept.permissions);
        (Some(started.model.name.clone()), mode)
    }
}

/// What a turn's configuration says it ran in, as far as it keeps it going on.
#[derive(Deserialize)]
struct Kept {
    permissions: Permissions,
}
