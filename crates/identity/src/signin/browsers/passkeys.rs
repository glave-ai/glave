//! Passkeys for the person (`specs/session.md`, "Signing in"): on their own server reached from
//! elsewhere, at its public origin, WebAuthn's relying party; on their own computer, at
//! `http://localhost` and the port the server listens on there alone. No password and no
//! email: elsewhere, glave on its computer makes a setup code, good once and for ten minutes,
//! which a browser there trades for a passkey it makes and a session with it; on this
//! computer, a browser signed in by glave's link makes one. After that, a passkey signs a
//! browser in, and someone signed in may make another. Each request to make or
//! use one is answered within five minutes, on the server that began it, and kept in memory
//! meanwhile, at most 16 of each. Setup codes that are not one are counted, and past 10 in a
//! minute none is taken until it passes. Removing a passkey ends the sessions it began.

use std::sync::{MutexGuard, PoisonError};

use glave_event::Secret;
use glave_ports::host::PublicOrigin;
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;
use webauthn_rs::prelude::{
    PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    Url, Webauthn, WebauthnBuilder,
};

use super::state::{Change, KeptPasskey, State};
use super::{BrowserError, Browsers, Session, new_id, random_hex};
use crate::signin::local::{digest, same};

/// How long a setup code may wait to be traded for a passkey.
const SETUP_LIFETIME: SignedDuration = SignedDuration::from_mins(10);

/// How long a request to make or use a passkey may wait for its answer.
const CEREMONY_LIFETIME: SignedDuration = SignedDuration::from_mins(5);

/// Requests of each kind waiting at once; a new one past this forgets the oldest.
const MOST_CEREMONIES: usize = 16;

/// Passkeys kept at once.
const MOST_PASSKEYS: usize = 32;

/// Setup codes that are not one taken in a minute before none is taken.
const MOST_MISSES: usize = 10;

/// What a setup code that is not one is counted against.
const MISS_WINDOW: SignedDuration = SignedDuration::from_mins(1);

/// The longest name a passkey keeps, in characters.
const LONGEST_NAME: usize = 64;

/// A request to make or use a passkey: its id, and the options the browser hands its
/// authenticator, as WebAuthn's JSON.
#[derive(Debug)]
pub struct Ceremony {
    pub id: String,
    pub options: serde_json::Value,
}

/// What proves the caller may make a passkey.
#[derive(Debug, Clone, Copy)]
pub enum Proof<'a> {
    /// A setup code glave on its computer made.
    Code(&'a str),
    /// The person, signed in already.
    SignedIn,
}

/// A passkey, as the person sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyInfo {
    pub id: String,
    pub name: String,
    pub added: Timestamp,
    pub used: Option<Timestamp>,
}

/// The relying party's id on the person's own computer.
pub(super) const HERE: &str = "localhost";

/// The relying party passkeys are made for, and the person they sign in.
pub(super) struct Relying {
    webauthn: Webauthn,
    person: Uuid,
    /// Its id: the public origin's name, or [`HERE`].
    id: String,
    party: Party,
}

/// Where a relying party's pages are.
enum Party {
    /// At the server's public origin.
    Public,
    /// On this computer, at this origin: `http://localhost:<port>`.
    Here(String),
}

impl Relying {
    /// The relying party at the public origin `origin`, for the server whose id is `server`.
    pub(super) fn public(origin: &PublicOrigin, server: &str) -> Result<Self, BrowserError> {
        Self::at(origin.as_str(), server, Party::Public)
    }

    /// The relying party for browsers on this computer: [`HERE`], at `port` alone, so a page
    /// another program serves at another port here is never one.
    fn here(port: u16, server: &str) -> Result<Self, BrowserError> {
        let origin = format!("http://{HERE}:{port}");
        Self::at(&origin, server, Party::Here(origin.clone()))
    }

    fn at(origin: &str, server: &str, party: Party) -> Result<Self, BrowserError> {
        let refused = |error: &dyn std::fmt::Display| BrowserError::Passkey(error.to_string());
        let url = Url::parse(origin).map_err(|e| refused(&e))?;
        let id = url.host_str().ok_or(BrowserError::NoPasskeys)?.to_owned();
        let webauthn = WebauthnBuilder::new(&id, &url)
            .and_then(|builder| builder.rp_name("glave").build())
            .map_err(|e| refused(&e))?;
        Ok(Self {
            webauthn,
            person: Uuid::parse_str(server).unwrap_or_default(),
            id,
            party,
        })
    }

    /// Whether `kept` was made for this relying party, so it signs browsers in with it.
    fn made(&self, kept: &KeptPasskey) -> bool {
        kept.made_for(&self.id, matches!(self.party, Party::Public))
    }
}

/// The passkey requests begun and not yet answered.
#[derive(Default)]
pub(super) struct Ceremonies {
    making: Vec<Making>,
    using: Vec<(Uuid, PasskeyAuthentication, Timestamp)>,
}

/// A request to make a passkey.
struct Making {
    id: Uuid,
    state: PasskeyRegistration,
    began: Timestamp,
    /// The digest of the setup code that proved it, if one did.
    code: Option<[u8; 32]>,
}

impl Browsers {
    /// Makes and uses passkeys for browsers on this computer, which reach the server as
    /// `http://localhost:<port>`, once it listens there on every address that name stands for:
    /// the relying party `localhost`, at that port alone. A server reached at a public origin
    /// keeps that one.
    pub fn reached_here(&self, port: u16) -> Result<(), BrowserError> {
        if self.relying.get().is_none() {
            // Set once: a second call, as of another listener, keeps the first.
            let _ = self.relying.set(Relying::here(port, &self.server)?);
        }
        Ok(())
    }

    /// Whether a browser signs in with a passkey now: at the public origin always, since a
    /// setup link makes the first; on this computer once one is made.
    pub fn signs_in_with_passkeys(&self) -> bool {
        match self.relying.get() {
            Some(Relying {
                party: Party::Public,
                ..
            }) => true,
            Some(relying) => self.state().passkeys.iter().any(|kept| relying.made(kept)),
            None => false,
        }
    }

    /// Where a browser on this computer makes and uses passkeys, when the server makes them
    /// here rather than at a public origin: `http://localhost:<port>`.
    pub fn passkeys_here(&self) -> Option<String> {
        match self.relying.get().map(|relying| &relying.party) {
            Some(Party::Here(origin)) => Some(origin.clone()),
            _ => None,
        }
    }

    /// A new setup code, for a link that makes a passkey elsewhere: good once, for ten minutes.
    pub fn setup(&self) -> Result<Secret, BrowserError> {
        self.relying()?;
        let code = random_hex()?;
        let now = self.clock.now();
        let mut state = self.state();
        state
            .setups
            .retain(|(_, made)| now.duration_since(*made) <= SETUP_LIFETIME);
        if state.setups.len() >= super::MOST_CODES {
            state.setups.remove(0);
        }
        state.setups.push((digest(&code), now));
        Ok(Secret::new(code))
    }

    /// Begins making a passkey, as `proof` allows.
    pub fn create_options(&self, proof: Proof<'_>) -> Result<Ceremony, BrowserError> {
        let relying = self.relying()?;
        let now = self.clock.now();
        let (code, exclude) = {
            let mut state = self.state();
            let code = match proof {
                Proof::Code(code) => Some(live_setup(&mut state, code, now)?),
                Proof::SignedIn => None,
            };
            if state.passkeys.len() >= MOST_PASSKEYS {
                return Err(BrowserError::Passkey(format!(
                    "this glave keeps at most {MOST_PASSKEYS} passkeys: remove one first"
                )));
            }
            let exclude = state
                .passkeys
                .iter()
                .filter(|kept| relying.made(kept))
                .map(|kept| kept.credential.cred_id().clone())
                .collect();
            (code, exclude)
        };
        let (options, registration) = relying
            .webauthn
            .start_passkey_registration(relying.person, "glave", "glave", Some(exclude))
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let id = new_id(now)?;
        let mut ceremonies = self.ceremonies();
        ceremonies
            .making
            .retain(|making| now.duration_since(making.began) <= CEREMONY_LIFETIME);
        if ceremonies.making.len() >= MOST_CEREMONIES {
            ceremonies.making.remove(0);
        }
        ceremonies.making.push(Making {
            id,
            state: registration,
            began: now,
            code,
        });
        ceremony(id, &options)
    }

    /// Makes the passkey `credential` answers the request `ceremony` with, called `name` if
    /// named, where a setup code proved the request: the code is spent, and the passkey signs
    /// this browser in.
    pub async fn create(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<(PasskeyInfo, Session), BrowserError> {
        let (passkey, change) = self.made(ceremony, credential, name, true)?;
        let id = parse(&passkey.id).ok_or(BrowserError::Ceremony)?;
        let session = self.begin(Some(id), vec![change]).await?;
        Ok((passkey, session))
    }

    /// Makes the passkey `credential` answers the request `ceremony` with, called `name` if
    /// named, where the person signed in made the request.
    pub async fn add(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<PasskeyInfo, BrowserError> {
        let (passkey, change) = self.made(ceremony, credential, name, false)?;
        self.change(|_, _| vec![change.clone()]).await?;
        Ok(passkey)
    }

    /// The passkey `credential` answers the request `ceremony` with, called `name` if named,
    /// and its record: a request a setup code proved when `by_code` says so, which spends it,
    /// else one the person signed in made.
    fn made(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
        by_code: bool,
    ) -> Result<(PasskeyInfo, Change), BrowserError> {
        let relying = self.relying()?;
        let now = self.clock.now();
        let making = {
            let mut ceremonies = self.ceremonies();
            let at = ceremonies
                .making
                .iter()
                .position(|making| {
                    Some(making.id) == parse(ceremony) && making.code.is_some() == by_code
                })
                .ok_or(BrowserError::Ceremony)?;
            ceremonies.making.remove(at)
        };
        if now.duration_since(making.began) > CEREMONY_LIFETIME {
            return Err(BrowserError::Ceremony);
        }
        let answer: RegisterPublicKeyCredential = serde_json::from_value(credential.clone())
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let passkey = relying
            .webauthn
            .finish_passkey_registration(&answer, &making.state)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        if let Some(code) = making.code {
            spend_setup(&mut self.state(), &code, now)?;
        }
        let id = new_id(now)?;
        let name = named(name, now);
        let kept = serde_json::to_string(&passkey)
            .map_err(|error| BrowserError::Keep(error.to_string()))?;
        let info = PasskeyInfo {
            id: id.to_string(),
            name: name.clone(),
            added: now,
            used: None,
        };
        let party = relying.id.clone();
        Ok((info, Change::PasskeyAdded(id, name, kept, party)))
    }

    /// Begins signing a browser in with a passkey: any of the person's made for this relying
    /// party.
    pub fn get_options(&self) -> Result<Ceremony, BrowserError> {
        let relying = self.relying()?;
        let passkeys: Vec<_> = self
            .state()
            .passkeys
            .iter()
            .filter(|kept| relying.made(kept))
            .map(|kept| kept.credential.clone())
            .collect();
        if passkeys.is_empty() {
            return Err(BrowserError::NoPasskey);
        }
        let (options, authentication) = relying
            .webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let now = self.clock.now();
        let id = new_id(now)?;
        let mut ceremonies = self.ceremonies();
        ceremonies
            .using
            .retain(|(_, _, began)| now.duration_since(*began) <= CEREMONY_LIFETIME);
        if ceremonies.using.len() >= MOST_CEREMONIES {
            ceremonies.using.remove(0);
        }
        ceremonies.using.push((id, authentication, now));
        ceremony(id, &options)
    }

    /// Signs a browser in with the passkey `credential` answers the request `ceremony` with.
    pub async fn get(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
    ) -> Result<Session, BrowserError> {
        let relying = self.relying()?;
        let now = self.clock.now();
        let (_, authentication, began) = {
            let mut ceremonies = self.ceremonies();
            let at = ceremonies
                .using
                .iter()
                .position(|(id, _, _)| Some(*id) == parse(ceremony))
                .ok_or(BrowserError::Ceremony)?;
            ceremonies.using.remove(at)
        };
        if now.duration_since(began) > CEREMONY_LIFETIME {
            return Err(BrowserError::Ceremony);
        }
        let answer: PublicKeyCredential = serde_json::from_value(credential.clone())
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let result = relying
            .webauthn
            .finish_passkey_authentication(&answer, &authentication)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let (id, changed) = {
            let state = self.state();
            let kept = state
                .passkeys
                .iter()
                .filter(|kept| relying.made(kept))
                .find(|kept| kept.credential.cred_id() == result.cred_id())
                .ok_or(BrowserError::NoPasskey)?;
            let mut credential = kept.credential.clone();
            let changed = credential.update_credential(&result) == Some(true);
            (kept.id, changed.then(|| serde_json::to_string(&credential)))
        };
        let changed = changed
            .transpose()
            .map_err(|error| BrowserError::Keep(error.to_string()))?;
        self.begin(Some(id), vec![Change::PasskeyUsed(id, changed)])
            .await
    }

    /// The person's passkeys, oldest first.
    pub fn passkeys(&self) -> Vec<PasskeyInfo> {
        self.state()
            .passkeys
            .iter()
            .map(|kept| PasskeyInfo {
                id: kept.id.to_string(),
                name: kept.name.clone(),
                added: kept.added,
                used: kept.used,
            })
            .collect()
    }

    /// Removes the passkey `id`, and ends the sessions it began.
    pub async fn remove(&self, id: &str) -> Result<(), BrowserError> {
        let Some(id) = parse(id) else {
            return Ok(());
        };
        self.change(|state, _| {
            if !state.passkeys.iter().any(|kept| kept.id == id) {
                return Vec::new();
            }
            let began: Vec<Uuid> = state
                .sessions
                .iter()
                .filter(|session| session.passkey == Some(id))
                .map(|session| session.id)
                .collect();
            let mut changes = vec![Change::PasskeyRemoved(id)];
            if !began.is_empty() {
                changes.push(Change::SignedOut(began));
            }
            changes
        })
        .await
    }

    fn relying(&self) -> Result<&Relying, BrowserError> {
        self.relying.get().ok_or(BrowserError::NoPasskeys)
    }

    fn ceremonies(&self) -> MutexGuard<'_, Ceremonies> {
        self.ceremonies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The digest of `code`, when it is a live setup code; counted against the limit when it is
/// not.
fn live_setup(
    state: &mut MutexGuard<'_, State>,
    code: &str,
    now: Timestamp,
) -> Result<[u8; 32], BrowserError> {
    state
        .misses
        .retain(|missed| now.duration_since(*missed) <= MISS_WINDOW);
    if state.misses.len() >= MOST_MISSES {
        return Err(BrowserError::TooMany);
    }
    let presented = digest(code);
    let found = state.setups.iter().position(|(kept, made)| {
        same(kept, &presented) && now.duration_since(*made) <= SETUP_LIFETIME
    });
    if found.is_none() {
        state.misses.push(now);
        return Err(BrowserError::Setup);
    }
    Ok(presented)
}

/// Spends the setup code whose digest is `code`, which must still be live.
fn spend_setup(
    state: &mut MutexGuard<'_, State>,
    code: &[u8; 32],
    now: Timestamp,
) -> Result<(), BrowserError> {
    let found = state
        .setups
        .iter()
        .position(|(kept, made)| same(kept, code) && now.duration_since(*made) <= SETUP_LIFETIME);
    let at = found.ok_or(BrowserError::Setup)?;
    state.setups.remove(at);
    Ok(())
}

/// The request `id`, its options as JSON.
fn ceremony(id: Uuid, options: &impl serde::Serialize) -> Result<Ceremony, BrowserError> {
    Ok(Ceremony {
        id: id.to_string(),
        options: serde_json::to_value(options)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?,
    })
}

/// A passkey's name: `name`, trimmed and cut short, else when it was added.
fn named(name: Option<&str>, now: Timestamp) -> String {
    let name: String = name
        .map(str::trim)
        .unwrap_or_default()
        .chars()
        .take(LONGEST_NAME)
        .collect();
    if name.is_empty() {
        format!("Passkey added {}", now.strftime("%Y-%m-%d"))
    } else {
        name
    }
}

fn parse(id: &str) -> Option<Uuid> {
    Uuid::parse_str(id).ok()
}

#[cfg(test)]
mod tests;
