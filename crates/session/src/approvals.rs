//! Who answers a thread's approvals and questions through the API. Where clients answer, a
//! turn stops at what it asks, the question already in the log (`approval.requested`, or the
//! `question` call), and holds nothing: a client answers with `approval/decide` or
//! `question/answer`, at any time and from any server of the deployment, the answer is recorded
//! in the log, and the turn goes on from it (`answers.rs`). With no client that can answer, as
//! in a script, every approval is answered "no one there" and `question` is not offered. The
//! same holds for a thread running as a task, and its subagents: no one is attached to answer.

use std::collections::HashSet;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::ThreadId;
use glave_harness::{ApprovalRequest, Approver, Decision};

use crate::hub::Hub;

/// Whether anyone can answer approvals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answering {
    /// Clients answer with `approval/decide`; a turn waits for them.
    Clients,
    /// No one can: every question is answered "no one there" at once, as `approvals.md`
    /// describes for scripts and CI.
    NoOne,
}

/// No one answers for a thread until this drops ([`Approvals::unattended_while`]).
pub(crate) struct Unattended {
    approvals: Approvals,
    thread: ThreadId,
}

impl Drop for Unattended {
    fn drop(&mut self) {
        self.approvals.attend(self.thread);
    }
}

/// Who answers each thread's approvals and questions.
#[derive(Clone)]
pub(crate) struct Approvals {
    answering: Answering,
    /// Threads running as tasks: no one is attached to answer them or their subagents.
    unattended: Arc<Mutex<HashSet<ThreadId>>>,
    /// Which thread started which, to find a subagent's task.
    hub: Arc<Hub>,
}

impl Approvals {
    pub(crate) fn new(answering: Answering, hub: Arc<Hub>) -> Self {
        Self {
            answering,
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

    /// No one answers for `thread` while what this returns lives: a turn, or a goal's turns,
    /// started with no one at its client.
    pub(crate) fn unattended_while(&self, thread: ThreadId) -> Unattended {
        self.unattend(thread);
        Unattended {
            approvals: self.clone(),
            thread,
        }
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
}

impl Approver for Approvals {
    fn can_answer(&self, thread: ThreadId) -> bool {
        self.answerable(thread)
    }

    fn waits(&self, thread: ThreadId) -> bool {
        self.answerable(thread)
    }

    /// Asked only where no one answers: wherever someone does, the turn waits in the log.
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::Unavailable)
    }
}
