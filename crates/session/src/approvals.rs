//! Approvals and questions answered through the API: the harness asks, the question is already
//! in the log (`approval.requested`, or the `question` call), and the turn waits until a client
//! answers with `approval/decide` or `question/answer`. With no client that can answer, as in a
//! script, every approval is answered "no one there" and `question` is not offered.

use std::collections::HashMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{CallId, PrincipalId, ThreadId};
use glave_harness::{ApprovalRequest, Approver, Decision, QuestionRequest};
use tokio::sync::oneshot;

use crate::protocol::{Answer, ApiError, code};

/// Whether anyone can answer approvals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answering {
    /// Clients answer with `approval/decide`; a turn waits for them.
    Clients,
    /// No one can: every question is answered "no one there" at once, as `approvals.md`
    /// describes for scripts and CI.
    NoOne,
}

/// A decision, and the person who made it when the caller was not the local person.
type Decided = (Decision, Option<PrincipalId>);
type Pending = HashMap<(ThreadId, CallId), oneshot::Sender<Decided>>;
type Questions = HashMap<(ThreadId, CallId), oneshot::Sender<Vec<String>>>;

/// The approvals waiting for an answer.
#[derive(Clone)]
pub(crate) struct Approvals {
    answering: Answering,
    pending: Arc<Mutex<Pending>>,
    questions: Arc<Mutex<Questions>>,
}

impl Approvals {
    pub(crate) fn new(answering: Answering) -> Self {
        Self {
            answering,
            pending: Arc::new(Mutex::new(HashMap::new())),
            questions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Answers the approval for `call` in `thread`, as `by` when that is a signed-in person.
    pub(crate) fn decide(
        &self,
        thread: ThreadId,
        call: &CallId,
        answer: Answer,
        by: Option<PrincipalId>,
    ) -> Result<(), ApiError> {
        let waiting = self
            .pending()
            .remove(&(thread, call.clone()))
            .ok_or_else(|| {
                ApiError::new(
                    code::NO_APPROVAL,
                    format!("no approval {} is pending", call.as_str()),
                )
            })?;
        let decision = match answer {
            Answer::AllowOnce => Decision::AllowOnce,
            Answer::AllowForThread => Decision::AllowForThread,
            Answer::Deny => Decision::Deny,
        };
        waiting.send((decision, by)).map_err(|_| {
            ApiError::new(
                code::NO_APPROVAL,
                format!("no approval {} is pending", call.as_str()),
            )
        })
    }

    fn questions(&self) -> std::sync::MutexGuard<'_, Questions> {
        self.questions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Answers the questions of call `question` in `thread`.
    pub(crate) fn respond(
        &self,
        thread: ThreadId,
        question: &CallId,
        answers: Vec<String>,
    ) -> Result<(), ApiError> {
        let unknown = || {
            ApiError::new(
                code::NO_QUESTION,
                format!("no question {} is waiting", question.as_str()),
            )
        };
        let waiting = self
            .questions()
            .remove(&(thread, question.clone()))
            .ok_or_else(unknown)?;
        waiting.send(answers).map_err(|_| unknown())
    }

    /// Whether `thread` has a turn paused for an answer.
    pub(crate) fn waiting(&self, thread: ThreadId) -> bool {
        self.pending().keys().any(|(waiting, _)| *waiting == thread)
            || self
                .questions()
                .keys()
                .any(|(waiting, _)| *waiting == thread)
    }

    /// Forgets `thread`'s questions once its turn has ended.
    pub(crate) fn forget(&self, thread: ThreadId) {
        self.pending().retain(|(waiting, _), _| *waiting != thread);
        self.questions()
            .retain(|(waiting, _), _| *waiting != thread);
    }
}

impl Approver for Approvals {
    fn can_answer(&self) -> bool {
        self.answering == Answering::Clients
    }

    fn answer(
        &self,
        request: &QuestionRequest,
    ) -> impl Future<Output = Option<Vec<String>>> + Send {
        let receiver = (self.answering == Answering::Clients).then(|| {
            let (sender, receiver) = oneshot::channel();
            self.questions()
                .insert((request.thread, request.call_id.clone()), sender);
            receiver
        });
        async move {
            match receiver {
                // Questions dropped without answers are unanswered.
                Some(receiver) => receiver.await.ok(),
                None => None,
            }
        }
    }

    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        let decided = self.decide_by(request);
        async move { decided.await.0 }
    }

    fn decide_by(&self, request: &ApprovalRequest) -> impl Future<Output = Decided> + Send {
        let receiver = match self.answering {
            Answering::NoOne => None,
            Answering::Clients => {
                let (sender, receiver) = oneshot::channel();
                self.pending()
                    .insert((request.thread, request.call_id.clone()), sender);
                Some(receiver)
            }
        };
        async move {
            match receiver {
                // A question dropped without an answer counts as declined.
                Some(receiver) => receiver.await.unwrap_or((Decision::Deny, None)),
                None => ready((Decision::Unavailable, None)).await,
            }
        }
    }
}
