//! Signing in with a passkey without first knowing whose it is, as a server with glave
//! accounts must (`specs/session.md`, "glave accounts for many people"): the request names no
//! passkeys, the browser offers every one it holds for the relying party, and the passkey's
//! user handle says whose it is. The request is answered within five minutes, on the server
//! that began it, at most 16 waiting.

use std::sync::{Mutex, PoisonError};

use glave_ports::host::PublicOrigin;
use jiff::Timestamp;
use uuid::Uuid;
use webauthn_rs::prelude::{DiscoverableAuthentication, DiscoverableKey, PublicKeyCredential};

use super::{CEREMONY_LIFETIME, Ceremony, MOST_CEREMONIES, Relying, ceremony, parse};
use crate::signin::browsers::state::Change;
use crate::signin::browsers::{BrowserError, Browsers, Session, new_id};

/// The requests to sign in with any passkey of the public origin, begun and not answered.
pub(crate) struct Discoverer {
    relying: Relying,
    waiting: Mutex<Vec<(Uuid, DiscoverableAuthentication, Timestamp)>>,
}

/// A passkey's answer to a request to sign in with any: whose it says it is, and the request.
pub struct Discovered {
    handle: Uuid,
    answer: PublicKeyCredential,
    state: DiscoverableAuthentication,
}

impl Discovered {
    /// The user handle the passkey says it was made for.
    pub fn handle(&self) -> Uuid {
        self.handle
    }
}

impl Discoverer {
    /// Requests for passkeys made at `origin`.
    pub(crate) fn new(origin: &PublicOrigin) -> Result<Self, BrowserError> {
        Ok(Self {
            relying: Relying::public(origin, Uuid::nil())?,
            waiting: Mutex::new(Vec::new()),
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
        let id = new_id(now)?;
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.retain(|(_, _, began)| now.duration_since(*began) <= CEREMONY_LIFETIME);
        if waiting.len() >= MOST_CEREMONIES {
            waiting.remove(0);
        }
        waiting.push((id, state, now));
        ceremony(id, &options)
    }

    /// Whose `credential`, answering the request `ceremony`, says it is: the request taken,
    /// refused when none waits, or it waited too long.
    pub(crate) fn identify(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        now: Timestamp,
    ) -> Result<Discovered, BrowserError> {
        let (_, state, began) = {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            let at = waiting
                .iter()
                .position(|(id, _, _)| Some(*id) == parse(ceremony))
                .ok_or(BrowserError::Ceremony)?;
            waiting.remove(at)
        };
        if now.duration_since(began) > CEREMONY_LIFETIME {
            return Err(BrowserError::Ceremony);
        }
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
            state,
        })
    }
}

impl Browsers {
    /// Signs a browser in with the passkey `discovered` answered with: one of this person's in
    /// the store as it is now, else [`BrowserError::Unknown`] for the browser to tell its
    /// passkey manager.
    pub async fn get_discovered(&self, discovered: Discovered) -> Result<Session, BrowserError> {
        let relying = self.relying()?;
        if discovered.handle != relying.handle {
            return Err(relying.unknown(&discovered.answer));
        }
        // Removed, or added, on another server on this store since this one last looked.
        self.catch_up().await?;
        let (id, mut kept) = self.kept(relying, &discovered.answer)?;
        let result = relying
            .webauthn
            .finish_discoverable_authentication(
                &discovered.answer,
                discovered.state,
                &[DiscoverableKey::from(&kept)],
            )
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let changed = (kept.update_credential(&result) == Some(true))
            .then(|| serde_json::to_string(&kept))
            .transpose()
            .map_err(|error| BrowserError::Keep(error.to_string()))?;
        self.begin(Some(id), vec![Change::PasskeyUsed(id, changed)])
            .await
    }
}
