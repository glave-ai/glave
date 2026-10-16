//! Renewing a GitHub App's token, which lasts eight hours, with its refresh token. One renewal
//! runs at a time in this process, as a task the host owns that runs to its end, so a command
//! that stops waiting never leaves a spent refresh token unkept. A renewal is kept only while
//! the credential it renews still is: a host removed or replaced meanwhile stays so, and its
//! renewed token is dropped. Another glave process on the same store may renew the same token
//! at once; whichever keeps its renewal first stands, and a command asks the ledger again
//! after any renewal ends, so it gets the one kept.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use glave_event::{CredentialConnected, CredentialId};
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::check::Http;
use super::device::Github;
use crate::accounts::Ledger;

/// How long after a renewal failed the same token's is tried again.
const RETRY: Duration = Duration::from_secs(60);

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
#[derive(Debug, Clone, Copy)]
struct Ended {
    /// Whether it got a new token, kept or dropped.
    renewed: bool,
    at: Instant,
}

/// One renewal: what its task needs, its own.
pub struct Renewing {
    pub ledger: Ledger,
    pub http: Http,
    pub github: Github,
    pub current: CredentialConnected,
}

impl Renewals {
    /// Waits for the renewal of `credential`: the one running, or one `renewing` gives; not at
    /// all when its last renewal failed under a minute ago, or `renewing` gives none.
    pub async fn wait(
        &self,
        credential: CredentialId,
        renewing: impl FnOnce() -> Option<Renewing>,
    ) {
        if let Some(mut ended) = self.joined(credential, renewing) {
            // Ends when the renewal does, or when its task was stopped.
            let _ = ended.wait_for(Option::is_some).await;
        }
    }

    /// Joins the renewal running, or starts one.
    fn joined(
        &self,
        credential: CredentialId,
        renewing: impl FnOnce() -> Option<Renewing>,
    ) -> Option<watch::Receiver<Option<Ended>>> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        while state.tasks.try_join_next().is_some() {}
        if let Some((renews, ended)) = &state.latest {
            let how = *ended.borrow();
            match how {
                None if !state.tasks.is_empty() => return Some(ended.clone()),
                Some(Ended { renewed: false, at })
                    if *renews == credential && at.elapsed() < RETRY =>
                {
                    return None;
                }
                _ => {}
            }
        }
        let job = renewing()?;
        let (tell, ended) = watch::channel(None);
        state.tasks.spawn(async move {
            let renewed = job.run().await;
            let at = Instant::now();
            let _ = tell.send(Some(Ended { renewed, at }));
        });
        state.latest = Some((credential, ended.clone()));
        Some(ended)
    }
}

impl Renewing {
    /// Renews the token and keeps the renewal in its place, while it is still kept. Whether
    /// GitHub renewed it.
    async fn run(self) -> bool {
        let url = &self.current.url;
        let Some(renewal) = &self.current.renewal else {
            return false;
        };
        let grant = match self.github.refresh(&self.http, &renewal.refresh).await {
            Ok(grant) => grant,
            Err(error) => {
                tracing::warn!("could not renew the sign-in to {url}: {}", error.message);
                return false;
            }
        };
        let fresh = match super::new_id() {
            Ok(id) => CredentialConnected {
                credential: id,
                token: grant.token,
                renewal: grant.renewal,
                ..self.current.clone()
            },
            Err(error) => {
                tracing::warn!("could not keep the renewed sign-in to {url}: {error:#}");
                return true;
            }
        };
        match self
            .ledger
            .renew_credential(self.current.credential, &fresh)
            .await
        {
            Ok(true) => {}
            Ok(false) => tracing::info!(
                "the sign-in to {url} was removed or replaced while it was renewed: the renewed \
                 token is not kept"
            ),
            Err(error) => {
                tracing::warn!("could not keep the renewed sign-in to {url}: {error:#}");
            }
        }
        true
    }
}
