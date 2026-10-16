//! Renewing a GitHub App's token, which lasts eight hours, with its refresh token. One renewal
//! runs at a time in this process, as a task the host owns that runs to its end while glave
//! runs, so a command that stops waiting never leaves a spent refresh token unkept. A refresh
//! token is spent once: a credential renewed, removed or replaced is never renewed again, and
//! a renewal glave could not keep is kept again, never renewed again. A renewal is kept only
//! while the credential it renews still is: a host removed or replaced meanwhile stays so, and
//! its renewed token is dropped. Another glave process on the same store may renew the same
//! token at once; whichever keeps its renewal first stands, and a command asks the ledger
//! again after any renewal ends, so it gets the one kept.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use glave_event::{CredentialConnected, CredentialId};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::check::Http;
use super::device::Github;
use crate::accounts::Ledger;

/// How long after a renewal failed the same token's is tried again, and after one ended the
/// least time before the next starts.
const RETRY: Duration = Duration::from_secs(60);
/// The longest a command waits for a renewal; the renewal goes on.
const WAIT: Duration = Duration::from_secs(30);
/// Times keeping a renewal is tried, and the pause between.
const KEEPS: usize = 3;
const KEEP_PAUSE: Duration = Duration::from_millis(500);

/// The renewals of one person's git hosts.
#[derive(Default)]
pub struct Renewals(Mutex<State>);

#[derive(Default)]
struct State {
    /// The renewal running, if one is, and any ended but not yet reaped.
    tasks: JoinSet<()>,
    /// The latest renewal: the credential it renews, and how it ended once it has.
    latest: Option<(CredentialId, watch::Receiver<Option<Ended>>)>,
}

/// How a renewal ended.
#[derive(Clone)]
enum Ended {
    /// Renewed and kept, or the credential was no longer kept: its refresh token is spent, or
    /// no longer glave's to spend.
    Done(Instant),
    /// GitHub did not renew it: the refresh token is unspent, and tried again after a while.
    Failed(Instant),
    /// Renewed, but glave could not keep the renewal: it serves commands from memory, and the
    /// next one to ask tries keeping it again.
    Unkept(Box<CredentialConnected>),
}

/// One renewal: what its task needs, its own.
pub struct Renewing {
    pub ledger: Ledger,
    pub http: Http,
    pub github: Github,
    pub current: CredentialConnected,
    /// A renewal already made but not kept, kept again rather than renewed again.
    pub fresh: Option<CredentialConnected>,
}

impl Renewals {
    /// Waits, up to half a minute, for the renewal of `credential`: the one running, or one
    /// `renewing` gives, handed any renewal made but not yet kept. None when one ended under a
    /// minute ago, `credential` was renewed already, or `renewing` gives none. The renewal
    /// it made when glave could not keep it, for this command to use.
    pub async fn wait(
        &self,
        credential: CredentialId,
        renewing: impl FnOnce(Option<CredentialConnected>) -> Option<Renewing>,
    ) -> Option<CredentialConnected> {
        let mut ended = self.joined(credential, renewing)?;
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

    /// Joins the renewal running, or starts one.
    fn joined(
        &self,
        credential: CredentialId,
        renewing: impl FnOnce(Option<CredentialConnected>) -> Option<Renewing>,
    ) -> Option<watch::Receiver<Option<Ended>>> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some(reaped) = state.tasks.try_join_next() {
            if let Err(error) = reaped {
                tracing::warn!("a renewal stopped before its end: {error}");
            }
        }
        let mut fresh = None;
        if let Some((renews, ended)) = &state.latest {
            let same = *renews == credential;
            let how = ended.borrow().clone();
            match how {
                None if !state.tasks.is_empty() => return Some(ended.clone()),
                Some(Ended::Done(_)) if same => return None,
                Some(Ended::Done(at) | Ended::Failed(at)) if at.elapsed() < RETRY => return None,
                Some(Ended::Unkept(unkept)) if same => fresh = Some(*unkept),
                _ => {}
            }
        }
        let job = renewing(fresh)?;
        let (tell, ended) = watch::channel(None);
        state.tasks.spawn(async move {
            let how = job.run().await;
            let _ = tell.send(Some(how));
        });
        state.latest = Some((credential, ended.clone()));
        Some(ended)
    }
}

impl Renewing {
    /// Renews the token, unless a renewal was made already, and keeps it in its place while
    /// the credential it renews is still kept.
    async fn run(self) -> Ended {
        let url = &self.current.url;
        let fresh = match self.fresh {
            Some(fresh) => fresh,
            None => match renewed(&self.ledger, &self.http, &self.github, &self.current).await {
                Ok(fresh) => fresh,
                Err(ended) => return ended,
            },
        };
        for attempt in 1..=KEEPS {
            match self
                .ledger
                .renew_credential(self.current.credential, &fresh)
                .await
            {
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

/// `current` renewed with its refresh token, under a new id, while it is still kept.
async fn renewed(
    ledger: &Ledger,
    http: &Http,
    github: &Github,
    current: &CredentialConnected,
) -> Result<CredentialConnected, Ended> {
    let url = &current.url;
    let failed = || Ended::Failed(Instant::now());
    let kept = match ledger.accounts().await {
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
    match github.refresh(http, &renewal.refresh).await {
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

#[cfg(test)]
mod tests;
