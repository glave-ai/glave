//! Renewing a sign-in whose token expires, with its refresh token: a GitHub App's, which lasts
//! eight hours, and a connection's (`connected.rs`). One renewal runs at a time for each
//! sign-in in this process, as a task the host owns that runs to its end while glave runs, so
//! a request that stops waiting never leaves a spent refresh token unkept; a request is only
//! ever handed what renewed its own sign-in. A refresh token is spent once: a sign-in renewed,
//! removed or replaced is never renewed again, and a renewal glave could not keep is kept
//! again, never renewed again. A renewal is kept only while the sign-in it renews still is:
//! one removed or replaced meanwhile stays so, and its renewed token is dropped. Another glave
//! process on the same store may renew the same token at once; whichever keeps its renewal
//! first stands, and a request asks the ledger again after any renewal ends, so it gets the
//! one kept.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use glave_event::{CredentialConnected, CredentialId};
use glave_session::AccountsError;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::check::Http;
use super::device::Github;
use crate::accounts::Ledger;

/// How long after a sign-in's renewal ended the next of its renewals may start.
const RETRY: Duration = Duration::from_secs(60);
/// The longest a request waits for a renewal; the renewal goes on.
const WAIT: Duration = Duration::from_secs(30);
/// Times keeping a renewal is tried, and the pause between.
const KEEPS: usize = 3;
const KEEP_PAUSE: Duration = Duration::from_millis(500);
/// How long before it expires a token is renewed, in seconds.
pub(crate) const RENEW_BEFORE: i64 = 5 * 60;

/// What renews one sign-in, `Kept` being how the ledger keeps it.
pub(crate) trait Renew: Send + 'static {
    type Kept: Clone + Send + Sync + 'static;

    /// Where the sign-in is, to say in the log.
    fn url(&self) -> &str;

    /// The sign-in renewed with its refresh token, under a new id, while it is still kept;
    /// how the renewal ended when it is not renewed.
    fn renewed(&self) -> impl Future<Output = Result<Self::Kept, Ended<Self::Kept>>> + Send;

    /// Keeps `fresh` in place of what it renews, while that is still kept: false when it no
    /// longer is.
    fn keep(&self, fresh: &Self::Kept) -> impl Future<Output = Result<bool, AccountsError>> + Send;
}

/// The renewals of one person's sign-ins of one kind.
pub(crate) struct Renewals<A>(Mutex<State<A>>);

impl<A> Default for Renewals<A> {
    fn default() -> Self {
        Self(Mutex::new(State {
            tasks: JoinSet::new(),
            renewals: BTreeMap::new(),
            started: BTreeMap::new(),
        }))
    }
}

struct State<A> {
    /// The renewals running, and any ended but not yet reaped.
    tasks: JoinSet<()>,
    /// Each sign-in's latest renewal, and how it ended once it has: kept while it runs, while
    /// it holds a renewal not kept, and for a while after it ended, so as many as the
    /// sign-ins renewed within that while.
    renewals: BTreeMap<CredentialId, Renewal<A>>,
    /// When a renewal last started for each address, within the last while. A sign-in renewed
    /// has a new id, and a token that lasts minutes is as soon due again: its address waits
    /// all the same, so a sign-in's renewals start at most once a while.
    started: BTreeMap<String, Instant>,
}

/// One sign-in's latest renewal.
struct Renewal<A> {
    started: Instant,
    ended: watch::Receiver<Option<Ended<A>>>,
}

/// How a renewal ended.
#[derive(Clone)]
pub(crate) enum Ended<A> {
    /// Renewed and kept, or the sign-in was no longer kept: its refresh token is spent, or no
    /// longer glave's to spend.
    Done(Instant),
    /// It was not renewed: the refresh token is unspent, and tried again after a while.
    Failed(Instant),
    /// Renewed, but glave could not keep the renewal: it serves requests from memory, and the
    /// next one to ask tries keeping it again.
    Unkept(Box<A>),
}

/// One renewal: what its task needs, its own.
pub(crate) struct Renewing<R: Renew> {
    pub job: R,
    /// A renewal already made but not kept, kept again rather than renewed again.
    pub fresh: Option<R::Kept>,
}

impl<A: Clone + Send + Sync + 'static> Renewals<A> {
    /// Waits, up to half a minute, for the renewal of `signed_in`: the one running, or one
    /// `renewing` gives, handed any renewal of it made but not yet kept. None when one of
    /// `signed_in` ended under a minute ago, or one at its address started then, or `renewing`
    /// gives none. The renewal it made when glave could not keep it, for this request to use.
    pub(crate) async fn wait<R: Renew<Kept = A>>(
        &self,
        signed_in: CredentialId,
        renewing: impl FnOnce(Option<A>) -> Option<Renewing<R>>,
    ) -> Option<A> {
        let mut ended = self.joined(signed_in, renewing)?;
        // Ends when the renewal does, or when its task was stopped.
        let waited = tokio::time::timeout(WAIT, ended.wait_for(Option::is_some)).await;
        let how = match waited {
            Ok(Ok(how)) => how.clone(),
            _ => None,
        };
        match how {
            Some(Ended::Unkept(fresh)) => Some(*fresh),
            _ => None,
        }
    }

    /// Forgets the renewals of sign-ins no longer `kept`, as a read of the ledger begun at
    /// `read` found them, removed here or by another glave: a renewal of one that glave could
    /// not keep goes with it, so its token is held nowhere. A renewal started since that read
    /// began stays, as the read may not show what it renews.
    pub(crate) fn only(&self, read: Instant, kept: impl Fn(CredentialId) -> bool) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state
            .renewals
            .retain(|signed_in, renewal| renewal.started >= read || kept(*signed_in));
    }

    /// Joins the renewal of `signed_in` running, or starts one.
    fn joined<R: Renew<Kept = A>>(
        &self,
        signed_in: CredentialId,
        renewing: impl FnOnce(Option<A>) -> Option<Renewing<R>>,
    ) -> Option<watch::Receiver<Option<Ended<A>>>> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some(reaped) = state.tasks.try_join_next() {
            if let Err(error) = reaped {
                tracing::warn!("a renewal stopped before its end: {error}");
            }
        }
        // A renewal that ended a while ago is needed no more: one that renewed left a new id,
        // and one asked again starts by reading whether its sign-in is still kept.
        // So is one whose task stopped before telling how it ended.
        state
            .renewals
            .retain(|_, renewal| match &*renewal.ended.borrow() {
                Some(Ended::Done(at) | Ended::Failed(at)) => at.elapsed() < RETRY,
                Some(Ended::Unkept(_)) => true,
                None => renewal.ended.has_changed().is_ok(),
            });
        state.started.retain(|_, at| at.elapsed() < RETRY);
        let mut fresh = None;
        if let Some(renewal) = state.renewals.get(&signed_in) {
            let ended = &renewal.ended;
            let how = ended.borrow().clone();
            match how {
                // Running, while its task can still tell how it ended.
                None if ended.has_changed().is_ok() => return Some(ended.clone()),
                Some(Ended::Done(_) | Ended::Failed(_)) => return None,
                Some(Ended::Unkept(unkept)) => fresh = Some(*unkept),
                None => {}
            }
        }
        let job = renewing(fresh)?;
        let url = job.job.url().to_owned();
        // Spending a refresh token waits its while; keeping a renewal made already does not.
        if job.fresh.is_none() && state.started.contains_key(&url) {
            return None;
        }
        let (tell, ended) = watch::channel(None);
        state.tasks.spawn(async move {
            let how = job.run().await;
            let _ = tell.send(Some(how));
        });
        let started = Instant::now();
        state.started.insert(url, started);
        let renewal = Renewal {
            started,
            ended: ended.clone(),
        };
        state.renewals.insert(signed_in, renewal);
        Some(ended)
    }
}

impl<R: Renew> Renewing<R> {
    /// Renews the token, unless a renewal was made already, and keeps it in its place while
    /// the sign-in it renews is still kept.
    async fn run(self) -> Ended<R::Kept> {
        let url = self.job.url().to_owned();
        let fresh = match self.fresh {
            Some(fresh) => fresh,
            None => match self.job.renewed().await {
                Ok(fresh) => fresh,
                Err(ended) => return ended,
            },
        };
        for attempt in 1..=KEEPS {
            match self.job.keep(&fresh).await {
                Ok(true) => return Ended::Done(Instant::now()),
                Ok(false) => {
                    tracing::info!(
                        "the sign-in to {url} was removed or replaced while it was renewed: \
                         the renewed token is not kept"
                    );
                    return Ended::Done(Instant::now());
                }
                Err(error) => {
                    tracing::warn!("could not keep the renewed sign-in to {url}: {error:#}");
                    if attempt < KEEPS {
                        tokio::time::sleep(KEEP_PAUSE).await;
                    }
                }
            }
        }
        Ended::Unkept(Box::new(fresh))
    }
}

/// Renewing a GitHub App's token.
pub(super) struct HostRenewal {
    pub ledger: Ledger,
    pub http: Http,
    pub github: Github,
    pub current: CredentialConnected,
}

impl Renew for HostRenewal {
    type Kept = CredentialConnected;

    fn url(&self) -> &str {
        &self.current.url
    }

    async fn renewed(&self) -> Result<CredentialConnected, Ended<CredentialConnected>> {
        let current = &self.current;
        let url = &current.url;
        let failed = || Ended::Failed(Instant::now());
        let kept = match self.ledger.accounts().await {
            Ok(accounts) => accounts
                .credentials
                .iter()
                .any(|kept| kept.credential == current.credential),
            Err(error) => {
                tracing::warn!("could not read the git hosts to renew {url}: {error:#}");
                return Err(failed());
            }
        };
        let (true, Some(renewal)) = (kept, &current.renewal) else {
            return Err(Ended::Done(Instant::now()));
        };
        // Made before the refresh token is spent, so nothing after can lose what it gets.
        let id = super::new_id().map_err(|error| {
            tracing::warn!("could not make an id to renew {url}: {error:#}");
            failed()
        })?;
        match self.github.refresh(&self.http, &renewal.refresh).await {
            Ok(grant) => Ok(CredentialConnected {
                credential: id,
                token: grant.token,
                renewal: grant.renewal,
                ..current.clone()
            }),
            Err(error) => {
                tracing::warn!("could not renew the sign-in to {url}: {}", error.message);
                Err(failed())
            }
        }
    }

    async fn keep(&self, fresh: &CredentialConnected) -> Result<bool, AccountsError> {
        self.ledger
            .renew_credential(self.current.credential, fresh)
            .await
    }
}

#[cfg(test)]
mod tests;
