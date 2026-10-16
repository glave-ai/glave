//! Handing a connected host's credential to a command that asks (`specs/computers.md`, rule 8):
//! read from the store for that one use. A GitHub App's token that is about to expire is
//! renewed first (`renewing.rs`), and the command gets whichever token is kept once that ends.

use std::sync::PoisonError;

use glave_event::CredentialConnected;
use glave_ports::{BoxFuture, CredentialAsk, CredentialHost, Credentials, GitCredential};

use super::Hosts;
use super::renewing::Renewing;

/// How long before it expires a token is renewed, in seconds.
const RENEW_BEFORE: i64 = 5 * 60;

impl Credentials for Hosts {
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
                let renewing = || self.renewing(&credential);
                self.renewals.wait(credential.credential, renewing).await;
                credential = still_working(self.kept_for(&url).await?)?;
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

    /// The renewal of `current`, when glave's GitHub App can renew it.
    fn renewing(&self, current: &CredentialConnected) -> Option<Renewing> {
        let github = self
            .github
            .as_ref()
            .filter(|github| github.url == current.url && current.renewal.is_some());
        let Some(github) = github else {
            tracing::warn!(
                "the sign-in to {} expires, and glave cannot renew it",
                current.url
            );
            return None;
        };
        Some(Renewing {
            ledger: self.ledger.clone(),
            http: self.http.clone(),
            github: github.clone(),
            current: current.clone(),
        })
    }
}

/// Seconds until `credential`'s token expires, for one that does.
fn left(credential: &CredentialConnected) -> Option<i64> {
    let renewal = credential.renewal.as_ref()?;
    let now = jiff::Timestamp::now().as_second();
    Some(renewal.expires.timestamp().as_second().saturating_sub(now))
}

/// `credential` while its token has not expired.
fn still_working(credential: CredentialConnected) -> Option<CredentialConnected> {
    left(&credential)
        .is_none_or(|left| left > 0)
        .then_some(credential)
}
