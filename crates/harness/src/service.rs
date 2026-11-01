//! What a connected service asks the person in the middle of a call (`specs/approvals.md`, "A
//! service asks"): a durable wait, as an approval is. The turn records what is asked and stops,
//! holding nothing; the answer is recorded in the log, and the turn goes on from it by making the
//! call again with that answer, past its hooks and approval, which it passed before it asked.
//! Where no one can answer, a source declines for them. What was typed, and what the source
//! keeps meanwhile, stay in blobs the events point to: neither a client nor the model reads them.

use glave_event::{
    BlobRef, Body, CallId, Event, Principal, PrincipalId, Seq, ServiceAnswer, ServiceAnswered,
    ServiceAsked, ServiceOutcome, ThreadId, misfit,
};
use glave_ports::{
    BlobError, BlobStore, EventLog, ModelProvider, Pieces, Scope, ServiceAsks, ServiceGiven,
    ToolOutput, WaitKind, read_all,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::approval::Approver;
use crate::calls::NOT_RUN;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Invalid, Thread};
use crate::turn::Live;
use crate::waits::by;

/// The most what a source keeps while the person answers may be.
const STATE_LIMIT: u64 = 1024 * 1024;

/// The most what a person answers one form with may be, an answer to each of its fields.
const ANSWERS_LIMIT: u64 = 1024 * 1024;

/// What is kept is padded to a multiple of this, so its size says little of what it holds.
const PADDED_TO: usize = 1024;

/// What is kept: `value`, with a salt of its own so no two have one hash.
#[derive(Serialize)]
struct Salted<'a, T> {
    salt: [u8; 10],
    value: &'a T,
}

/// What [`Salted`] reads back as.
#[derive(Deserialize)]
struct KeptBack<T> {
    value: T,
}

/// What the latest thing a service asked in a call came to.
#[derive(Debug, Clone)]
pub(crate) struct Round {
    pub(crate) asked: ServiceAsked,
    pub(crate) answered: Option<ServiceAnswered>,
}

/// What the call `call` in `turn` last asked, with its answer when one came: `None` when it has
/// asked nothing.
pub(crate) fn round_of(turn: &[Event], call: &CallId) -> Option<Round> {
    let at = turn.iter().rposition(
        |event| matches!(&event.body, Body::ServiceAsked(asked) if asked.call_id == *call),
    )?;
    let after = turn.get(at..)?;
    let mut events = after.iter().map(|event| &event.body);
    let Some(Body::ServiceAsked(asked)) = events.next() else {
        return None;
    };
    let answered = events.find_map(|body| match body {
        Body::ServiceAnswered(answered) if answered.call_id == *call => Some(answered.clone()),
        _ => None,
    });
    Some(Round {
        asked: asked.clone(),
        answered,
    })
}

/// What the person was shown of the field `name` of what was `asked`.
fn label_of(asked: &ServiceAsked, name: &str) -> String {
    let field = asked.fields.iter().find(|field| field.name == name);
    field.map_or(name, |field| field.label.as_str()).to_owned()
}

/// What a person reads of who asks: so a tool cannot pass as the model.
fn asks_sentence(service: &str) -> String {
    format!("{service} asks you, not the model")
}

/// Where the answer goes.
fn note_sentence(service: &str) -> String {
    format!(
        "Your answer goes only to {service}. The model sees that you answered, not what you typed."
    )
}

/// What came of it, in a line.
fn told(service: &str, outcome: ServiceOutcome) -> String {
    match outcome {
        ServiceOutcome::Answered => format!("You answered {service}"),
        ServiceOutcome::Declined => format!("You declined {service}'s question"),
        ServiceOutcome::Cancelled => format!("{service}'s question went unanswered"),
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Whether what a service asks in `thread` can reach a person: not a subagent's, whose
    /// parent's person isn't following it, and not where no one answers.
    pub(crate) fn someone_answers(&self, thread: &Thread) -> bool {
        !thread.delegated() && self.approver.waits(thread.id) && self.approver.can_answer(thread.id)
    }

    /// Stores `value` for `thread`, as nothing an event's reader can read or guess: salted, so the
    /// hash an event holds does not give away a yes or a short answer, and padded, so its size
    /// does not.
    async fn keep(
        &self,
        scope: &Scope,
        thread: ThreadId,
        value: &impl Serialize,
        limit: u64,
    ) -> Result<BlobRef, HarnessError> {
        let salted = Salted {
            salt: self.entropy.bytes()?,
            value,
        };
        let mut bytes =
            serde_json::to_vec(&salted).map_err(|e| HarnessError::Event(e.to_string()))?;
        bytes.resize(bytes.len().next_multiple_of(PADDED_TO), b' ');
        let stored = self.blobs.put(
            scope,
            Some(thread),
            "application/json",
            limit,
            Box::new(Pieces::from(bytes)),
        );
        Ok(stored.await?)
    }

    /// What `blob`, stored by [`Self::keep`], holds.
    async fn kept_back<T: DeserializeOwned>(
        &self,
        scope: &Scope,
        blob: &BlobRef,
        limit: u64,
    ) -> Result<T, HarnessError> {
        let found = self.blobs.get(scope, &blob.sha256).await?;
        let lost = || HarnessError::Event(format!("{} is gone", blob.sha256.as_str()));
        let mut found = found.ok_or_else(lost)?;
        let bytes = read_all(found.bytes.as_mut(), limit).await?;
        let kept: KeptBack<T> =
            serde_json::from_slice(&bytes).map_err(|e| HarnessError::Event(e.to_string()))?;
        Ok(kept.value)
    }

    /// Records what a service asks, with what its source keeps meanwhile, for `call` to wait at:
    /// the turn stops here (`HarnessError::Waits`). What keeps more than glave holds fails its
    /// call, with a line the model can read.
    pub(crate) async fn record_asked(
        &self,
        thread: &mut Thread,
        call: &CallId,
        asks: ServiceAsks,
    ) -> Result<ToolOutput, HarnessError> {
        let kept = self
            .keep(&thread.scope, thread.id, &asks.state, STATE_LIMIT)
            .await;
        let state = match kept {
            Err(HarnessError::Blob(BlobError::TooLarge(_))) => {
                let service = &asks.service;
                let said = format!(
                    "{service} asked the person something, but keeps more with it than glave \
                     holds while they answer, so the call stopped."
                );
                return Ok(ToolOutput::text(true, said));
            }
            kept => kept?,
        };
        let asked = ServiceAsked {
            call_id: call.clone(),
            ask: asks_sentence(&asks.service),
            note: note_sentence(&asks.service),
            service: asks.service,
            message: asks.message,
            fields: asks.fields,
            state,
        };
        self.record(thread, &asked).await?;
        Err(HarnessError::Waits(thread.id))
    }

    /// What a call that waited at `round` goes on with, its answer read back: the result the
    /// model is given instead when the turn was stopped meanwhile, which settles what still
    /// waits; refused as waiting while no answer came.
    pub(crate) async fn given(
        &self,
        thread: &mut Thread,
        round: Round,
        cancel: &Cancel,
        summary: &str,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<Result<ServiceGiven, ToolOutput>, HarnessError> {
        let Round { asked, answered } = round;
        let stopped = cancel.is_cancelled();
        if answered.is_none() && !stopped {
            return Err(HarnessError::Waits(thread.id));
        }
        let answer = match answered {
            None => {
                let settled = ServiceAnswered {
                    call_id: asked.call_id,
                    told: told(&asked.service, ServiceOutcome::Cancelled),
                    service: asked.service,
                    outcome: ServiceOutcome::Cancelled,
                    answers: None,
                    by: Some(Principal::glave()),
                };
                self.record(thread, &settled).await?;
                None
            }
            Some(_) if stopped => None,
            Some(answered) => match (answered.outcome, answered.answers) {
                (ServiceOutcome::Answered, Some(blob)) => Some(ServiceAnswer::Answered {
                    values: self.kept_back(&thread.scope, &blob, ANSWERS_LIMIT).await?,
                }),
                (ServiceOutcome::Declined, _) => Some(ServiceAnswer::Declined),
                (ServiceOutcome::Answered | ServiceOutcome::Cancelled, _) => None,
            },
        };
        let Some(answer) = answer else {
            live(Live::ToolFinished {
                summary: summary.to_owned(),
                is_error: true,
            });
            return Ok(Err(ToolOutput::text(true, NOT_RUN)));
        };
        let state = self
            .kept_back(&thread.scope, &asked.state, STATE_LIMIT)
            .await?;
        Ok(Ok(ServiceGiven { state, answer }))
    }

    /// Records `answer`, by `person` when that is not the thread's own, to what `call` in thread
    /// `id` waits for at `asked`, exactly as the turn waiting for it would: refused when no turn
    /// there waits for it, as when it asked something else since, or another answer came first,
    /// and when it does not fit what was asked. The turn goes on from the log
    /// ([`Harness::go_on`]).
    #[expect(
        clippy::too_many_arguments,
        reason = "where it goes, what it answers, the answer, and who gave it"
    )]
    pub async fn answer_service(
        &self,
        scope: Scope,
        id: ThreadId,
        call: &CallId,
        asked: Seq,
        answer: ServiceAnswer,
        person: Option<PrincipalId>,
    ) -> Result<(), HarnessError> {
        let refused = || HarnessError::NoServiceAsk(call.clone());
        let (view, wait) = self.waiting(&scope, id, call, WaitKind::Service).await?;
        let wait = wait.filter(|wait| *wait == asked).ok_or_else(refused)?;
        let Body::ServiceAsked(asked) = self.event_at(&scope, id, wait).await?.body else {
            return Err(refused());
        };
        let (outcome, answers) = match &answer {
            ServiceAnswer::Answered { values } => {
                if let Some(field) = misfit(&asked.fields, values) {
                    return Err(Invalid::ServiceAnswer(label_of(&asked, field)).into());
                }
                let kept = self.keep(&scope, id, values, ANSWERS_LIMIT).await;
                let blob = kept.map_err(|error| match error {
                    HarnessError::Blob(BlobError::TooLarge(_)) => {
                        let longest = values
                            .iter()
                            .max_by_key(|(_, value)| value.to_string().len());
                        let field = longest.map_or("", |(name, _)| name.as_str());
                        Invalid::ServiceAnswer(label_of(&asked, field)).into()
                    }
                    other => other,
                })?;
                (ServiceOutcome::Answered, Some(blob))
            }
            ServiceAnswer::Declined => (ServiceOutcome::Declined, None),
        };
        let answered = ServiceAnswered {
            call_id: call.clone(),
            told: told(&asked.service, outcome),
            service: asked.service,
            outcome,
            answers,
            by: by(&view, person),
        };
        self.answer_with(scope, id, &view, &answered, refused).await
    }
}
