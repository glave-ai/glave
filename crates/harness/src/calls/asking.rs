//! Asking about a call: the judge first in auto mode, then the person, and what their answer
//! means for the call.

use glave_event::{
    ApprovalDecided, ApprovalRequested, Caution, Principal, Settled, TurnResult, Verdict,
};
use glave_ports::settings::Judge;
use glave_ports::{BlobStore, EventLog, ModelProvider, ToolOutput};

use super::{NOT_IN_PLAN, NOT_RUN};
use crate::approval::{ApprovalMode, ApprovalRequest, Approver, asks_for, no_one_there};
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::judge::{JudgeCase, Ruling, judge};
use crate::tools::Tool;
use crate::turn::Live;
use crate::waits::{Answered, verdict_of};

const DECLINED: &str = "The person declined this action. Ask them what they would like \
     instead, or take a different approach.";

const DECLINED_SECRET: &str = "Declined: this would show you secrets such as keys, tokens or \
     passwords. Do not try to get them another way. If you need a specific value, ask the \
     person for it.";

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Asks the person about a call when it needs them: for its caution, unless the judge clears
    /// it, or for the mode; Full access asks no one ([`Self::ask`]). `None` when the call may run;
    /// otherwise the result the model is given instead.
    pub(crate) async fn approve(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        turn: &mut TurnResult,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let mut cautioned = request.caution.is_some();
        if cautioned {
            match self.judged(thread, request, turn, cancel).await? {
                Judged::Cleared => cautioned = false,
                Judged::Ask => {}
                Judged::Cancelled => return Ok(Some(ToolOutput::text(true, NOT_RUN))),
            }
        }
        // A command that stays in the thread's sandbox, which Ask first lets run: `bash` is
        // always the built-in, the first source that names a tool keeping it.
        let confined = request.tool == Tool::Bash.name()
            && request.outside_sandbox.is_none()
            && self.workspace_for(thread).sandbox().is_some();
        // A caution asks even when the tool was allowed for the rest of the thread; in plan
        // mode, where it is refused, nothing allowed before counts.
        let allowed = thread.allowed.contains(&request.tool) && !self.planning(thread);
        let asks = cautioned
            || (asks_for(self.approval_for(thread), request.effect, confined) && !allowed);
        if !asks {
            return Ok(None);
        }
        self.ask(thread, request, cancel, live).await
    }

    /// The model that judges `thread`'s flagged actions, or none when no judge does, so the
    /// person is asked: the trusted project's `judge_model`, else the harness's (the person's
    /// or the server's `judge_model`). The main model, as judge, is the default model the
    /// person's or the server's settings give, never the model the thread's turns go to: that
    /// can be the project's `model`, which a repository sets without being trusted, and so
    /// can the model a turn read again from its log kept. With no default model, none judges.
    fn judge_for(&self, thread: &Thread) -> Option<String> {
        let settled = &thread.brought.settled;
        match settled.judge.as_ref().unwrap_or(&thread.defaults.judge) {
            Judge::Off => None,
            Judge::Main => Some(self.default_model()).filter(|model| !model.is_empty()),
            Judge::Model(model) => Some(model.clone()),
        }
    }

    /// In auto mode with a judge, asks it whether a caution may run because the person
    /// asked for it. The judge's call is recorded and counted in the turn, as model usage this
    /// thread caused; a cleared action is recorded as an approval glave decided, its question
    /// and answer in one write.
    async fn judged(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        turn: &mut TurnResult,
        cancel: &Cancel,
    ) -> Result<Judged, HarnessError> {
        let Some(reason) = request.caution.as_ref().and_then(Caution::because) else {
            return Ok(Judged::Ask);
        };
        let Some(model) = self.judge_for(thread) else {
            return Ok(Judged::Ask);
        };
        if self.approval_for(thread) != ApprovalMode::Risky {
            return Ok(Judged::Ask);
        }
        let case = JudgeCase {
            asked: &thread.asked,
            action: &request.summary,
            flagged: &reason,
            untrusted: thread.untrusted,
            outside_sandbox: request.outside_sandbox.as_ref().map(|o| o.lifted),
            secret: request.secret,
        };
        let asking = judge(&self.provider, &model, Some(thread.payer()), &case);
        let Some((ruling, answer)) = cancel.run(asking).await else {
            return Ok(Judged::Cancelled);
        };
        if let Some(answer) = &answer {
            turn.model_calls = turn.model_calls.saturating_add(1);
            turn.take_in(&answer.responded());
            self.record(thread, &answer.responded()).await?;
        }
        let Ruling::Run { reason: why } = ruling else {
            return Ok(Judged::Ask);
        };
        // Why it asks, then the judge's own words for why it need not.
        let why = Some(why.trim().to_owned()).filter(|why| !why.is_empty());
        let asked = request.settled(Settled::Cleared { why });
        let decided = ApprovalDecided {
            call_id: request.call_id.clone(),
            decision: Verdict::AllowOnce,
            by: Some(Principal::glave()),
        };
        // Written together, so that no stop between them can leave it waiting for anyone.
        let mut events = Vec::new();
        self.stage(thread, &mut events, &asked)?;
        self.stage(thread, &mut events, &decided)?;
        self.commit(thread, events).await?;
        Ok(Judged::Cleared)
    }

    /// Asks the person about `request`, recording the question and the answer. `None` when the
    /// call may go ahead; otherwise the result the model is given instead. Full access asks no
    /// one: the call goes ahead. Plan mode asks no one either: what would ask is refused, since
    /// nothing changes until the person approves the plan.
    pub(crate) async fn ask(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        if self.approval_for(thread) == ApprovalMode::Never {
            return Ok(None);
        }
        if self.planning(thread) {
            live(Live::ToolFinished {
                summary: format!("{} ({NOT_IN_PLAN})", request.summary),
                is_error: true,
            });
            return Ok(Some(ToolOutput::text(true, crate::plan::refused(request))));
        }
        self.put_to_person(thread, request, cancel, live).await
    }

    /// Puts `request` to the person, whatever the mode, recording the question and the answer:
    /// `None` when the call may go ahead; otherwise the result the model is given instead. Where
    /// the person answers in the log, the turn stops here and goes on from their answer.
    pub(crate) async fn put_to_person(
        &self,
        thread: &mut Thread,
        request: &ApprovalRequest,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let asked = request.asked();
        self.record(thread, &asked).await?;
        if self.approver.waits(thread.id) {
            // The turn stops here: the answer comes to the log, and the turn goes on from it.
            live(Live::ApprovalPending {
                call_id: request.call_id.clone(),
            });
            return Err(HarnessError::Waits(thread.id));
        }
        let deciding = self.approver.decide_by(request);
        // Pending now, so a client told of it can answer it at once.
        if self.approver.can_answer(thread.id) {
            live(Live::ApprovalPending {
                call_id: request.call_id.clone(),
            });
        }
        let decided = cancel.run(deciding).await;
        let (decision, person) = decided.map_or((None, None), |(d, by)| (Some(d), by));
        // Running outside the sandbox is allowed one command at a time.
        let verdict = decision.map_or(Verdict::Cancelled, |decision| {
            verdict_of(decision, request.outside_sandbox.is_some())
        });
        // The person decides, unless glave did: no one was there, or the turn was cancelled.
        let by = if matches!(verdict, Verdict::NoOneThere | Verdict::Cancelled) {
            Some(Principal::glave())
        } else {
            Self::other_than_for(thread, person)
        };
        crate::telemetry::approval(&request.tool, verdict);
        let decided = ApprovalDecided {
            call_id: request.call_id.clone(),
            decision: verdict,
            by,
        };
        self.record(thread, &decided).await?;
        let no_one = (verdict == Verdict::NoOneThere).then(|| no_one_there(request));
        Ok(settle(thread, &asked, verdict, no_one, live))
    }

    /// Takes in the answer to the approval a turn going on from the log stopped at: the one
    /// recorded for it, or, once the turn was stopped, a cancellation recorded now. `None` when
    /// the call may go ahead; otherwise the result the model is given instead. With no answer
    /// yet and the turn not stopped, it waits on.
    pub(crate) async fn answered(
        &self,
        thread: &mut Thread,
        answered: Answered,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Option<ToolOutput>, HarnessError> {
        let verdict = match answered.decided {
            Some(decided) => decided.decision,
            None if cancel.is_cancelled() => {
                let decided = ApprovalDecided {
                    call_id: answered.asked.call_id.clone(),
                    decision: Verdict::Cancelled,
                    by: Some(Principal::glave()),
                };
                crate::telemetry::approval(&answered.asked.tool, decided.decision);
                self.record(thread, &decided).await?;
                Verdict::Cancelled
            }
            None => return Err(HarnessError::Waits(thread.id)),
        };
        Ok(settle(thread, &answered.asked, verdict, None, live))
    }
}

/// What `verdict` on the approval `asked` means for its call: `None` when it may run; otherwise
/// the result the model is given instead, `no_one` when no one was there to answer.
fn settle(
    thread: &mut Thread,
    asked: &ApprovalRequested,
    verdict: Verdict,
    no_one: Option<String>,
    live: &mut (dyn FnMut(Live) + Send),
) -> Option<ToolOutput> {
    // The model is about to see a file that usually holds secrets.
    if asked.secret && matches!(verdict, Verdict::AllowOnce | Verdict::AllowForThread) {
        thread.read_secret = true;
    }
    match verdict {
        Verdict::AllowOnce => None,
        Verdict::AllowForThread => {
            thread.allowed.insert(asked.tool.clone());
            None
        }
        Verdict::Cancelled => Some(ToolOutput::text(true, NOT_RUN)),
        Verdict::Deny | Verdict::NoOneThere => {
            live(Live::ToolDeclined {
                summary: asked.summary.clone(),
                caution: asked.caution.clone(),
                reason: asked.reason.clone(),
                no_one_there: verdict == Verdict::NoOneThere,
            });
            let declined = if asked.secret {
                DECLINED_SECRET
            } else {
                DECLINED
            };
            Some(ToolOutput::text(
                true,
                no_one.unwrap_or_else(|| declined.to_owned()),
            ))
        }
    }
}

/// What the judge made of a caution.
enum Judged {
    /// The person asked for it: it runs without asking.
    Cleared,
    /// The person decides, as for any caution.
    Ask,
    /// The turn was cancelled while the judge thought.
    Cancelled,
}
