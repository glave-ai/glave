//! Approvals and questions answered through the API: the harness asks, the question is already
//! in the log (`approval.requested`, or the `question` call), and the turn waits until a client
//! answers with `approval/decide` or `question/answer`. With no client that can answer, as in a
//! script, every approval is answered "no one there" and `question` is not offered. The same
//! holds for a thread running as a task, and its subagents: no one is attached to answer.

use std::collections::{HashMap, HashSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{CallId, PrincipalId, ThreadId};
use glave_harness::{ApprovalRequest, Approver, Decision, QuestionRequest};
use tokio::sync::oneshot;

use crate::hub::Hub;
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
    /// Threads running as tasks: no one is attached to answer them or their subagents.
    unattended: Arc<Mutex<HashSet<ThreadId>>>,
    /// Which thread started which, to find a subagent's task.
    hub: Arc<Hub>,
}

impl Approvals {
    pub(crate) fn new(answering: Answering, hub: Arc<Hub>) -> Self {
        Self {
            answering,
            pending: Arc::new(Mutex::new(HashMap::new())),
            questions: Arc::new(Mutex::new(HashMap::new())),
            unattended: Arc::new(Mutex::new(HashSet::new())),
            hub,
        }
    }

    fn unattended(&self) -> std::sync::MutexGuard<'_, HashSet<ThreadId>> {
        self.unattended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether clients answer here at all, as the person at an app does.
    pub(crate) fn clients_answer(&self) -> bool {
        self.answering == Answering::Clients
    }

    /// From now on no one answers for `thread`, which runs as a task.
    pub(crate) fn unattend(&self, thread: ThreadId) {
        self.unattended().insert(thread);
    }

    /// `thread`'s task ended: clients answer for it again.
    pub(crate) fn attend(&self, thread: ThreadId) {
        self.unattended().remove(&thread);
    }

    /// Whether a client may answer for `thread`: someone can, and neither it nor a thread
    /// that started it runs as a task.
    fn answerable(&self, thread: ThreadId) -> bool {
        if self.answering == Answering::NoOne {
            return false;
        }
        let unattended = self.unattended();
        if unattended.is_empty() {
            return true;
        }
        !self
            .hub
            .lineage(thread)
            .iter()
            .any(|thread| unattended.contains(thread))
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
    fn can_answer(&self, thread: ThreadId) -> bool {
        self.answerable(thread)
    }

    fn answer(
        &self,
        request: &QuestionRequest,
    ) -> impl Future<Output = Option<Vec<String>>> + Send {
        let receiver = self.answerable(request.thread).then(|| {
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
        let receiver = self.answerable(request.thread).then(|| {
            let (sender, receiver) = oneshot::channel();
            self.pending()
                .insert((request.thread, request.call_id.clone()), sender);
            receiver
        });
        async move {
            match receiver {
                // A question dropped without an answer counts as declined.
                Some(receiver) => receiver.await.unwrap_or((Decision::Deny, None)),
                None => ready((Decision::Unavailable, None)).await,
            }
        }
    }
}
