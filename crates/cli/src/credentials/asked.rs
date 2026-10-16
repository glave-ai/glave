//! Handing a connected host's credential to a command that asks (`specs/computers.md`, rule 8):
//! read from the store for that one use. A GitHub App's token that is about to expire is
//! renewed first, once at a time, and the renewed one is kept sealed in its place.

use std::sync::PoisonError;

use glave_event::CredentialConnected;
use glave_ports::{BoxFuture, CredentialAsk, CredentialHost, Credentials, GitCredential};

use super::Hosts;

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
                credential = self.renewed(&url).await?;
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

    /// The credential for `url` renewed through GitHub's app, one renewal at a time; the token
    /// as it is while it still works when renewing fails, and none once it has expired.
    async fn renewed(&self, url: &str) -> Option<CredentialConnected> {
        let _one = self.renewing.lock().await;
        // Another request may have renewed it while this one waited.
        let current = self.kept_for(url).await?;
        if left(&current).is_none_or(|left| left >= RENEW_BEFORE) {
            return Some(current);
        }
        let renewed = match (&self.github, &current.renewal) {
            (Some(github), Some(renewal)) if github.url == current.url => {
                github.refresh(&self.http, &renewal.refresh).await
            }
            _ => {
                tracing::warn!("the sign-in to {url} expires, and glave cannot renew it");
                return still_working(current);
            }
        };
        let grant = match renewed {
            Ok(grant) => grant,
            Err(error) => {
                tracing::warn!("could not renew the sign-in to {url}: {}", error.message);
                return still_working(current);
            }
        };
        let fresh = self
            .made(
                current.forge,
                current.url.clone(),
                current.username.clone(),
                current.login.clone(),
                grant,
            )
            .ok()?;
        if let Err(error) = self.keep(fresh.clone()).await {
            // The old refresh token is spent: the renewed token still serves this use.
            tracing::warn!(
                "could not keep the renewed sign-in to {url}: {}",
                error.message
            );
        }
        Some(fresh)
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
