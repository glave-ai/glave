//! Signing in with a passkey without first knowing whose it is, as a server with glave
//! accounts must (`specs/session.md`, "glave accounts for many people"): the request names no
//! passkeys, the browser offers every one it holds for the relying party, and the passkey's
//! user handle says whose it is. The browser holds the request, sealed (`sealed`), and it is
//! answered within five minutes, once, on the server that began it.

use std::sync::Arc;

use glave_ports::host::PublicOrigin;
use jiff::Timestamp;
use serde::de::IgnoredAny;
use uuid::Uuid;
use webauthn_rs::prelude::{DiscoverableAuthentication, DiscoverableKey, PublicKeyCredential};

use super::sealed::{Kind, Request, Sealer};
use super::{Ceremony, Relying, ceremony};
use crate::signin::browsers::{BrowserError, Browsers, Session};

/// Requests to sign in with any passkey of the public origin.
pub(crate) struct Discoverer {
    relying: Relying,
    sealer: Arc<Sealer>,
}

/// A passkey's answer to a request to sign in with any: whose it says it is, and the request.
pub struct Discovered {
    handle: Uuid,
    answer: PublicKeyCredential,
    state: DiscoverableAuthentication,
    /// The request's id and when it began, spent once a passkey answered it.
    spent: (Uuid, Timestamp),
    sealer: Arc<Sealer>,
}

impl Discovered {
    /// The user handle the passkey says it was made for.
    pub fn handle(&self) -> Uuid {
        self.handle
    }

    /// The credential id the passkey answered with.
    pub(crate) fn credential(&self) -> &[u8] {
        self.answer.get_credential_id()
    }
}

impl Discoverer {
    /// Requests for passkeys made at `origin`, sealed under `sealer`.
    pub(crate) fn new(origin: &PublicOrigin, sealer: Arc<Sealer>) -> Result<Self, BrowserError> {
        Ok(Self {
            relying: Relying::public(origin, Uuid::nil())?,
            sealer,
        })
    }

    /// Begins a request for any passkey, at `now`.
    pub(crate) fn options(&self, now: Timestamp) -> Result<Ceremony, BrowserError> {
        let (mut options, state) = self
            .relying
            .webauthn
            .start_discoverable_authentication()
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        // Asked for at once, as the sign-in page asks, not in the browser's autofill.
        options.mediation = None;
        let request = Request::new(Kind::Any, Uuid::nil(), now, state)?;
        ceremony(self.sealer.seal(&request)?, &options)
    }

    /// Whether the request `ceremony` is one of these, at `now`.
    pub(crate) fn began(&self, ceremony: &str, now: Timestamp) -> bool {
        self.sealer
            .open::<IgnoredAny>(ceremony, Kind::Any, now)
            .is_ok()
    }

    /// Whose `credential`, answering the request `ceremony`, says it is: refused when the
    /// request is not one of these, or it waited too long.
    pub(crate) fn identify(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        now: Timestamp,
    ) -> Result<Discovered, BrowserError> {
        let request: Request<DiscoverableAuthentication> =
            self.sealer.open(ceremony, Kind::Any, now)?;
        let answer: PublicKeyCredential = serde_json::from_value(credential.clone())
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let (handle, _) = self
            .relying
            .webauthn
            .identify_discoverable_authentication(&answer)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        Ok(Discovered {
            handle,
            answer,
            state: request.state,
            spent: (request.id, request.began),
            sealer: Arc::clone(&self.sealer),
        })
    }

    /// Why `discovered` is refused when no glave account here knows its passkey: the same
    /// whether or not its handle is an account's.
    pub(crate) fn no_account_knows(&self, discovered: &Discovered) -> BrowserError {
        self.relying.no_account_knows(&discovered.answer)
    }
}

impl Browsers {
    /// Signs a browser in with the passkey `discovered` answered with: one of this person's in
    /// the store as it is now, else [`BrowserError::Unknown`] for the browser to tell its
    /// passkey manager. The request is spent once the passkey's signature is checked.
    pub async fn get_discovered(&self, discovered: Discovered) -> Result<Session, BrowserError> {
        let relying = self.relying()?;
        if discovered.handle != relying.handle {
            return Err(relying.unknown(&discovered.answer));
        }
        // Removed, or added, on another server on this store since this one last looked.
        self.catch_up().await?;
        let (id, kept) = self.kept(relying, &discovered.answer)?;
        let result = relying
            .webauthn
            .finish_discoverable_authentication(
                &discovered.answer,
                discovered.state,
                &[DiscoverableKey::from(&kept)],
            )
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let now = self.clock.now();
        discovered
            .sealer
            .spend(discovered.handle, discovered.spent, now)?;
        self.used(id, kept, &result).await
    }
}
