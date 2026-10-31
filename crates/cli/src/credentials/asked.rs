//! Handing a connected host's credential to a command that asks (`specs/computers.md`, rule 8):
//! read from the store for that one use. A GitHub App's token that is about to expire is
//! renewed first (`renewing.rs`), and the command gets whichever token is kept once that ends.

use std::sync::PoisonError;

use glave_event::CredentialConnected;
use glave_ports::computer::{CredentialAsk, CredentialHost, GitCredential, GitCredentials};
use glave_ports::{BoxFuture, Clock as _, SystemClock};

use super::Hosts;
use super::device::Github;
use super::renewing::{HostRenewal, RENEW_BEFORE, Renewing};

impl GitCredentials for Hosts {
    fn hosts(&self) -> BoxFuture<'_, Vec<CredentialHost>> {
        let offered = self
            .offered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Box::pin(std::future::ready(offered))
    }

    fn request<'a>(&'a self, ask: &'a CredentialAsk) -> BoxFuture<'a, Option<GitCredential>> {
        Box::pin(async move {
            let url = format!("{}://{}", ask.protocol, ask.host.to_ascii_lowercase());
            let mut credential = self.kept_for(&url).await?;
            if left(&credential).is_some_and(|left| left < RENEW_BEFORE) {
                let renewing = |fresh| self.renewing(&credential, fresh);
                let unkept = self.renewals.wait(credential.credential, renewing).await;
                let kept = self.kept_for(&url).await?;
                // A renewal glave could not keep serves while the ledger holds what it renewed.
                credential = match unkept {
                    Some(fresh) if kept.credential == credential.credential => fresh,
                    _ => kept,
                };
                credential = still_working(credential)?;
            }
            GitCredential::new(credential.username, credential.token.expose()).ok()
        })
    }
}

impl Hosts {
    /// The credential kept for the host at `url`, if one is.
    async fn kept_for(&self, url: &str) -> Option<CredentialConnected> {
        match self.ledger.accounts().await {
            Ok(accounts) => accounts
                .credentials
                .into_iter()
                .find(|credential| credential.url == url),
            Err(error) => {
                tracing::warn!("could not read the git hosts for a command: {error:#}");
                None
            }
        }
    }

    /// Whether `credential`'s token has expired with nothing left to renew it: only signing in
    /// again brings it back.
    pub(super) fn expired(credential: &CredentialConnected) -> bool {
        left(credential).is_some_and(|left| left <= 0) && Github::issuer(credential).is_none()
    }

    /// The renewal of `current`, through the app that issued it, when it can be renewed, keeping
    /// `fresh` when it was renewed already.
    fn renewing(
        &self,
        current: &CredentialConnected,
        fresh: Option<CredentialConnected>,
    ) -> Option<Renewing<HostRenewal>> {
        let Some(github) = Github::issuer(current) else {
            tracing::info!(
                "the sign-in to {} expires, and cannot be renewed",
                current.url
            );
            return None;
        };
        let job = HostRenewal {
            ledger: self.ledger.clone(),
            http: self.http.clone(),
            github,
            current: current.clone(),
        };
        Some(Renewing { job, fresh })
    }
}

/// Seconds until `credential`'s token expires, for one that does.
fn left(credential: &CredentialConnected) -> Option<i64> {
    let renewal = credential.renewal.as_ref()?;
    let now = SystemClock.now().as_second();
    Some(renewal.expires.timestamp().as_second().saturating_sub(now))
}

/// `credential` while its token has not expired.
fn still_working(credential: CredentialConnected) -> Option<CredentialConnected> {
    left(&credential)
        .is_none_or(|left| left > 0)
        .then_some(credential)
}
