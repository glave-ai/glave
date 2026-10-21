//! Trusting a project from an app (`specs/session.md`, "Project trust"). When the person on
//! this computer opens or starts a thread in a project whose folder brings what runs only once
//! they trust it (`specs/policy.md`), the thread asks them, in its summary, until they answer:
//! trusting keeps the answer where the terminal keeps it, and turns on what was listed from
//! the thread's next turn; not now keeps it off and stops asking in that thread. What a
//! project asks is read again from its files and the person's answers every time, so it is
//! never stale, and a server that restarts asks again when the thread is opened. A caller
//! signed in from elsewhere is never asked: what a project runs, runs on this computer.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{MutexGuard, PoisonError};

use glave_event::ThreadId;
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider, ProjectId};

use crate::protocol::{ApiError, Reply, Status, ThreadSummary, TrustAsk, TrustChoice, code};
use crate::providers::local;
use crate::server::Server;

/// Where the host reads what a project asks to be trusted, and keeps the person's yes.
pub trait ProjectTrust: Send + Sync {
    /// What the project whose folder is `folder` asks to be trusted now, if anything.
    fn ask<'a>(&'a self, folder: &'a Path) -> BoxFuture<'a, Result<Option<TrustAsk>, ApiError>>;

    /// Trusts what the ask named `ask` showed of the project whose folder is `folder`, and
    /// keeps the answer; `false`, trusting nothing, when it no longer asks exactly that.
    fn trust<'a>(&'a self, folder: &'a Path, ask: &'a str)
    -> BoxFuture<'a, Result<bool, ApiError>>;
}

/// The threads whose asks are remembered; past this the oldest is forgotten, and asks again
/// when it is opened.
const REMEMBERED: usize = 4_096;

/// What a thread did with its project's ask.
enum Said {
    /// It was opened or started since the ask: it shows the ask until it is answered.
    Raised,
    /// The person said not now to the ask of this name.
    NotNow(String),
}

/// The asks of the threads opened or started here.
#[derive(Default)]
pub(crate) struct Asks {
    said: HashMap<ThreadId, Said>,
    order: VecDeque<ThreadId>,
}

impl Asks {
    fn set(&mut self, thread: ThreadId, said: Said) {
        if self.said.insert(thread, said).is_none() {
            self.order.push_back(thread);
        }
        while self.order.len() > REMEMBERED {
            if let Some(oldest) = self.order.pop_front() {
                self.said.remove(&oldest);
            }
        }
    }

    /// Whether `thread` shows the ask named `ask`: it was opened or started since, or is being
    /// now (`raising`), and the person did not say not now to it.
    fn shows(&self, thread: ThreadId, ask: &str, raising: bool) -> bool {
        match self.said.get(&thread) {
            Some(Said::NotNow(said)) if said == ask => false,
            Some(Said::Raised) => true,
            _ => raising,
        }
    }

    /// Whether any of `threads` shows an ask, whatever it names.
    fn any_raised(&self, mut threads: impl Iterator<Item = ThreadId>) -> bool {
        threads.any(|thread| matches!(self.said.get(&thread), Some(Said::Raised)))
    }
}

/// Refused: nothing in the thread's project waits to be trusted.
fn nothing_waits() -> ApiError {
    ApiError::new(
        code::NO_TRUST,
        "nothing in this thread's project waits to be trusted",
    )
}

/// Refused: the project asks something else now than what the person was shown.
fn changed() -> ApiError {
    ApiError::new(
        code::NO_TRUST,
        "this project changed since you were asked: look again at what it brings now",
    )
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    fn asks(&self) -> MutexGuard<'_, Asks> {
        self.inner
            .asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// What the project of `project` asks `caller` to trust now: only for the person on this
    /// computer, where someone answers, in a project with a folder here.
    pub(crate) async fn trust_in(&self, caller: &Caller, project: ProjectId) -> Option<TrustAsk> {
        let host = self.inner.trust.as_ref()?;
        if !local(caller) || !self.inner.approvals.clients_answer() {
            return None;
        }
        let folder = self.folder_here(project).await.ok().flatten()?;
        match host.ask(Path::new(&folder)).await {
            Ok(ask) => ask,
            Err(error) => {
                tracing::warn!(%error, "could not read what a project asks to be trusted");
                None
            }
        }
    }

    /// What a page of `threads` in `project` shows `caller` of its project's ask: read only when
    /// one of them was opened or started since an ask.
    pub(crate) async fn trust_on_page(
        &self,
        caller: &Caller,
        project: ProjectId,
        threads: impl Iterator<Item = ThreadId>,
    ) -> Option<TrustAsk> {
        if !self.asks().any_raised(threads) {
            return None;
        }
        self.trust_in(caller, project).await
    }

    /// `summary` with its project's ask `ask`, when it shows it, and waiting while it does.
    /// `raising`: the thread is being opened or started, which raises the ask in it.
    pub(crate) fn asking(
        &self,
        mut summary: ThreadSummary,
        ask: Option<&TrustAsk>,
        raising: bool,
    ) -> ThreadSummary {
        let Some(ask) = ask else {
            return summary;
        };
        let mut asks = self.asks();
        if !asks.shows(summary.id, &ask.ask, raising) {
            return summary;
        }
        let raised = raising && !matches!(asks.said.get(&summary.id), Some(Said::Raised));
        if raising {
            asks.set(summary.id, Said::Raised);
        }
        drop(asks);
        // It waits for the person from now: lists are told.
        if raised {
            self.inner.hub.changed(summary.project, summary.id);
        }
        summary.trust = Some(ask.clone());
        if summary.status == Status::Idle {
            summary.status = Status::Waiting;
        }
        summary
    }

    /// Answers `trust/answer` from `caller`: the thread's summary after it, which asks
    /// nothing more.
    pub(crate) async fn answer_trust(
        &self,
        caller: &Caller,
        thread: ThreadId,
        ask: &str,
        answer: TrustChoice,
    ) -> Result<Reply, ApiError> {
        if !local(caller) {
            return Err(ApiError::new(
                code::NOT_ALLOWED,
                "only the person on this computer can trust a project: what it brings runs \
                 on this computer",
            ));
        }
        if !self.inner.approvals.clients_answer() {
            return Err(nothing_waits());
        }
        let scope = self.scope_of(thread).await?;
        let host = self.inner.trust.as_ref().ok_or_else(nothing_waits)?;
        let folder = self
            .folder_here(scope.project())
            .await?
            .ok_or_else(nothing_waits)?;
        let folder = Path::new(&folder);
        let current = host.ask(folder).await?.ok_or_else(nothing_waits)?;
        if current.ask != ask {
            return Err(changed());
        }
        match answer {
            TrustChoice::Trust => {
                if !host.trust(folder, ask).await? {
                    return Err(changed());
                }
            }
            TrustChoice::NotNow => self.asks().set(thread, Said::NotNow(ask.to_owned())),
        }
        // It waits no more: lists are told.
        self.inner.hub.changed(scope.project(), thread);
        Ok(Reply::Thread {
            thread: self.summary(scope, thread).await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(n: u128) -> ThreadId {
        let random: [u8; 16] = n.to_be_bytes();
        let mut tail = [0; 10];
        tail.copy_from_slice(&random[6..]);
        ThreadId::new(jiff::Timestamp::UNIX_EPOCH, tail).unwrap()
    }

    #[test]
    fn a_thread_shows_an_ask_once_raised_until_it_says_not_now_to_it() {
        let mut asks = Asks::default();
        let one = thread(1);
        assert!(!asks.shows(one, "a", false), "not opened since");
        assert!(asks.shows(one, "a", true), "opened now");
        asks.set(one, Said::Raised);
        assert!(asks.shows(one, "a", false));
        asks.set(one, Said::NotNow("a".into()));
        assert!(
            !asks.shows(one, "a", true),
            "not now, even when opened again"
        );
        assert!(asks.shows(one, "b", true), "a changed project asks again");
        assert!(!asks.any_raised([one].into_iter()));
    }

    #[test]
    fn the_oldest_threads_are_forgotten_past_the_cap() {
        let mut asks = Asks::default();
        for n in 0..=u128::try_from(REMEMBERED).unwrap() {
            asks.set(thread(n), Said::NotNow("a".into()));
        }
        assert_eq!(asks.said.len(), REMEMBERED);
        assert!(asks.shows(thread(0), "a", true), "forgotten: asks again");
        assert!(!asks.shows(thread(1), "a", true));
    }
}
