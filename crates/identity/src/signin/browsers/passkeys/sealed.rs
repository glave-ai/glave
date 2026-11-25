//! Passkey requests the browser holds, not the server: each one's state, what it is for, whose
//! it is and when it began, sealed (AES-256-GCM) under a key made at random as the server
//! starts and never kept, is the request's id. So nothing a stranger asks for is kept, and no
//! one's asking pushes anyone else's request out. Sealed, the state can't be read or changed;
//! what webauthn-rs warns of when its state leaves the server (`danger-allow-state-serialisation`)
//! is a request answered twice, so each is answered within its five minutes and once: a sign-in
//! answered is recorded as spent until it would have expired, for the person whose passkey
//! answered it, at most [`MOST_SPENT`] each. Making a passkey spends what proved it instead: a
//! setup code, an invite or an emailed link. A request is answered on the server that began it,
//! whose key alone opens it.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use aws_lc_rs::aead::{AES_256_GCM, Aad, NONCE_LEN, Nonce, RandomizedNonceKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::Timestamp;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::CEREMONY_LIFETIME;
use crate::signin::browsers::{BrowserError, new_id};

/// Sign-ins answered that one person's passkeys may have spent within a request's lifetime:
/// past it, theirs wait until the oldest expires. Only a passkey of theirs spends one.
const MOST_SPENT: usize = 256;

/// What a passkey request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Kind {
    /// Making a passkey for the person whose handle it names.
    Make,
    /// Signing in with one of the person's passkeys, which it names.
    Use,
    /// Signing in with any passkey of the public origin, whose its handle says.
    Any,
}

/// A passkey request as the browser holds it, opened.
#[derive(Serialize, Deserialize)]
pub(super) struct Request<S> {
    kind: Kind,
    pub(super) id: Uuid,
    /// The person it is for: nil for [`Kind::Any`].
    pub(super) handle: Uuid,
    pub(super) began: Timestamp,
    /// The digest of the setup code that proved a [`Kind::Make`], if one did.
    pub(super) code: Option<[u8; 32]>,
    /// What the caller asked it to carry to its answer.
    pub(super) carried: Option<serde_json::Value>,
    pub(super) state: S,
}

impl<S> Request<S> {
    /// A new request of `kind` for `handle` at `now`.
    pub(super) fn new(
        kind: Kind,
        handle: Uuid,
        now: Timestamp,
        state: S,
    ) -> Result<Self, BrowserError> {
        Ok(Self {
            kind,
            id: new_id(now)?,
            handle,
            began: now,
            code: None,
            carried: None,
            state,
        })
    }
}

/// The key passkey requests are sealed under on this server, and the sign-ins spent.
pub(crate) struct Sealer {
    key: RandomizedNonceKey,
    /// By the handle of the person whose passkey answered: each request spent, and when it
    /// began.
    spent: Mutex<BTreeMap<Uuid, Vec<(Uuid, Timestamp)>>>,
}

impl Sealer {
    /// A new key, made at random.
    pub(crate) fn new() -> Result<Self, BrowserError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| BrowserError::Random(e.to_string()))?;
        let key = RandomizedNonceKey::new(&AES_256_GCM, &bytes)
            .map_err(|_| BrowserError::Random("no key for passkey requests".into()))?;
        Ok(Self {
            key,
            spent: Mutex::new(BTreeMap::new()),
        })
    }

    /// `request`, sealed: its id as the browser holds it.
    pub(super) fn seal<S: Serialize>(&self, request: &Request<S>) -> Result<String, BrowserError> {
        let mut sealed = serde_json::to_vec(request)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let nonce = self
            .key
            .seal_in_place_append_tag(aad(request.kind), &mut sealed)
            .map_err(|_| BrowserError::Passkey("a passkey request could not be sealed".into()))?;
        let mut held = nonce.as_ref().to_vec();
        held.extend(sealed);
        Ok(URL_SAFE_NO_PAD.encode(held))
    }

    /// The request of `kind` the browser holds as `id`, opened at `now`: refused alike when it
    /// is not one sealed here, is of another kind, or waited too long.
    pub(super) fn open<S: DeserializeOwned>(
        &self,
        id: &str,
        kind: Kind,
        now: Timestamp,
    ) -> Result<Request<S>, BrowserError> {
        let held = URL_SAFE_NO_PAD
            .decode(id)
            .map_err(|_| BrowserError::Ceremony)?;
        let (nonce, sealed) = held
            .split_at_checked(NONCE_LEN)
            .ok_or(BrowserError::Ceremony)?;
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| BrowserError::Ceremony)?;
        let mut sealed = sealed.to_vec();
        let opened = self
            .key
            .open_in_place(nonce, aad(kind), &mut sealed)
            .map_err(|_| BrowserError::Ceremony)?;
        let request: Request<S> =
            serde_json::from_slice(opened).map_err(|_| BrowserError::Ceremony)?;
        let fresh = now.duration_since(request.began) <= CEREMONY_LIFETIME;
        if request.kind != kind || !fresh {
            return Err(BrowserError::Ceremony);
        }
        Ok(request)
    }

    /// What the request to make a passkey the browser holds as `id` carries, opened at `now`:
    /// none when it is not one sealed here, carries nothing, or waited too long.
    pub(crate) fn carried(&self, id: &str, now: Timestamp) -> Option<serde_json::Value> {
        self.open::<IgnoredAny>(id, Kind::Make, now)
            .ok()
            .and_then(|request| request.carried)
    }

    /// Records the sign-in `id`, begun at `began`, as spent by the passkey of the person whose
    /// handle is `handle`, at `now`: refused when it was spent already, or that person's
    /// passkeys spent [`MOST_SPENT`] within a request's lifetime.
    pub(super) fn spend(
        &self,
        handle: Uuid,
        (id, began): (Uuid, Timestamp),
        now: Timestamp,
    ) -> Result<(), BrowserError> {
        let mut spent = self.spent.lock().unwrap_or_else(PoisonError::into_inner);
        spent.retain(|_, theirs| {
            theirs.retain(|(_, began)| now.duration_since(*began) <= CEREMONY_LIFETIME);
            !theirs.is_empty()
        });
        let theirs = spent.entry(handle).or_default();
        if theirs.len() >= MOST_SPENT || theirs.iter().any(|(spent, _)| *spent == id) {
            return Err(BrowserError::Ceremony);
        }
        theirs.push((id, began));
        Ok(())
    }
}

/// What a request of `kind` is sealed with beside it, so one is never opened as another.
fn aad(kind: Kind) -> Aad<[u8; 1]> {
    Aad::from([match kind {
        Kind::Make => b'm',
        Kind::Use => b'u',
        Kind::Any => b'a',
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).unwrap()
    }

    fn fresh() -> Uuid {
        new_id(at(0)).unwrap()
    }

    #[test]
    fn a_request_opens_as_its_own_kind_within_its_lifetime_and_never_changed() {
        let sealer = Sealer::new().unwrap();
        let handle = fresh();
        let request = Request::new(Kind::Use, handle, at(0), "state".to_owned()).unwrap();
        let id = sealer.seal(&request).unwrap();
        let opened: Request<String> = sealer.open(&id, Kind::Use, at(300)).unwrap();
        assert_eq!((opened.handle, opened.state.as_str()), (handle, "state"));
        assert!(
            sealer.open::<String>(&id, Kind::Make, at(0)).is_err(),
            "another kind"
        );
        assert!(
            sealer.open::<String>(&id, Kind::Use, at(301)).is_err(),
            "expired"
        );
        let other = Sealer::new().unwrap();
        assert!(
            other.open::<String>(&id, Kind::Use, at(0)).is_err(),
            "another server"
        );
        let mut changed = URL_SAFE_NO_PAD.decode(&id).unwrap();
        if let Some(last) = changed.last_mut() {
            *last ^= 1;
        }
        let changed = URL_SAFE_NO_PAD.encode(changed);
        assert!(
            sealer.open::<String>(&changed, Kind::Use, at(0)).is_err(),
            "changed"
        );
    }

    #[test]
    fn a_sign_in_is_spent_once_and_a_person_s_passkeys_spend_at_most_their_own() {
        let sealer = Sealer::new().unwrap();
        let (one, other) = (fresh(), fresh());
        let first = (fresh(), at(0));
        sealer.spend(one, first, at(1)).unwrap();
        assert!(sealer.spend(one, first, at(2)).is_err(), "answered twice");
        for _ in 1..MOST_SPENT {
            sealer.spend(one, (fresh(), at(10)), at(10)).unwrap();
        }
        assert!(sealer.spend(one, (fresh(), at(10)), at(10)).is_err());
        assert!(
            sealer.spend(other, (fresh(), at(10)), at(10)).is_ok(),
            "another's"
        );
        assert!(
            sealer.spend(one, (fresh(), at(301)), at(301)).is_ok(),
            "the first expired"
        );
    }
}
