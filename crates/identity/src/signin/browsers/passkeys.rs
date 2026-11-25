//! Passkeys for the person (`specs/session.md`, "Signing in"): on their own server reached from
//! elsewhere, at its public origin, WebAuthn's relying party; on their own computer, at
//! `http://localhost` and the port the server listens on there alone. No password and no
//! email: elsewhere, glave on its computer makes a setup code, good once and for ten minutes,
//! which a browser there trades for a passkey it makes and a session with it; on this
//! computer, a browser signed in by glave's link makes one. After that, a passkey signs a
//! browser in, and someone signed in may make another. Each request to make or use one is
//! held by the browser, sealed (`sealed`), and answered within five minutes, once, on the
//! server that began it, which keeps nothing of it meanwhile. A setup code, 32 random bytes,
//! is never guessed: one that is not one is refused, and no more. Removing a passkey ends the
//! sessions it began. Every passkey carries the person's handle, which the caller gives, so a
//! passkey made again on a device replaces the one made there before; and the browser is told
//! which of the person's passkeys still sign in, and of one that does not, so its passkey
//! manager forgets the rest (`signals`).

mod discoverable;
mod sealed;
mod signals;

pub use self::discoverable::Discovered;
pub(crate) use self::discoverable::Discoverer;
pub(crate) use self::sealed::Sealer;

pub use self::signals::{AcceptedPasskeys, UnknownPasskey};

use std::sync::MutexGuard;

use glave_event::Secret;
use glave_ports::host::PublicOrigin;
use jiff::{SignedDuration, Timestamp};
use uuid::Uuid;
use webauthn_rs::prelude::{
    AuthenticationResult, DiscoverableAuthentication, DiscoverableKey, Passkey,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential, Url, Webauthn,
    WebauthnBuilder, WebauthnError,
};

use self::sealed::{Kind, Request};

use super::state::{Change, KeptPasskey, State};
use super::{BrowserError, Browsers, Session, new_id, random_hex};
use crate::signin::local::{digest, same};

/// How long a setup code may wait to be traded for a passkey.
const SETUP_LIFETIME: SignedDuration = SignedDuration::from_mins(10);

/// How long a request to make or use a passkey may wait for its answer.
const CEREMONY_LIFETIME: SignedDuration = SignedDuration::from_mins(5);

/// Passkeys kept at once.
const MOST_PASSKEYS: usize = 32;

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
    /// The person, signed in already; or, for the first passkey of a new glave account, the
    /// invite they opened, which the server checked before asking.
    SignedIn,
}

/// A passkey made and checked, not kept yet ([`Browsers::checked`]).
pub struct Checked {
    info: PasskeyInfo,
    change: Change,
}

/// A passkey, as the person sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyInfo {
    pub id: String,
    pub name: String,
    pub added: Timestamp,
    pub used: Option<Timestamp>,
    /// Whether it was made from a link sent to the account's email, to get back in.
    pub emailed: bool,
}

/// The relying party's id on the person's own computer.
pub(super) const HERE: &str = "localhost";

/// The relying party passkeys are made for, and the person they sign in.
pub(super) struct Relying {
    webauthn: Webauthn,
    /// The person's user handle, which each of their passkeys keeps.
    handle: Uuid,
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
    /// The relying party at the public origin `origin`, for the person whose handle is
    /// `handle`.
    pub(super) fn public(origin: &PublicOrigin, handle: Uuid) -> Result<Self, BrowserError> {
        Self::at(origin.as_str(), handle, Party::Public)
    }

    /// The relying party for browsers on this computer: [`HERE`], at `port` alone, so a page
    /// another program serves at another port here is never one.
    fn here(port: u16, handle: Uuid) -> Result<Self, BrowserError> {
        let origin = format!("http://{HERE}:{port}");
        Self::at(&origin, handle, Party::Here(origin.clone()))
    }

    fn at(origin: &str, handle: Uuid, party: Party) -> Result<Self, BrowserError> {
        let refused = |error: &dyn std::fmt::Display| BrowserError::Passkey(error.to_string());
        let url = Url::parse(origin).map_err(|e| refused(&e))?;
        let id = url.host_str().ok_or(BrowserError::NoPasskeys)?.to_owned();
        let webauthn = WebauthnBuilder::new(&id, &url)
            .and_then(|builder| builder.rp_name("glave").build())
            .map_err(|e| refused(&e))?;
        Ok(Self {
            webauthn,
            handle,
            id,
            party,
        })
    }

    /// Whether `kept` was made for this relying party, so it signs browsers in with it.
    fn made(&self, kept: &KeptPasskey) -> bool {
        kept.made_for(&self.id, matches!(self.party, Party::Public))
    }
}

impl Browsers {
    /// Makes and uses passkeys for browsers on this computer, which reach the server as
    /// `http://localhost:<port>`, once it listens there on every address that name stands for:
    /// the relying party `localhost`, at that port alone. A server reached at a public origin
    /// keeps that one.
    pub fn reached_here(&self, port: u16) -> Result<(), BrowserError> {
        if self.relying.get().is_none() {
            // Set once: a second call, as of another listener, keeps the first.
            let _ = self.relying.set(Relying::here(port, self.handle)?);
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

    /// Whether `kept` was made at the server's public origin, so the sessions it began sign
    /// in through that origin too. Told by the relying party in use, never by its name: people
    /// who share a computer reach it at `http://localhost:<port>`, the name its own browsers use.
    pub(super) fn made_public(&self, kept: &KeptPasskey) -> bool {
        self.relying
            .get()
            .is_some_and(|relying| matches!(relying.party, Party::Public) && relying.made(kept))
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
        self.create_options_carrying(proof, None)
    }

    /// Begins making a passkey, as `proof` allows, the request carrying `carried` to its answer
    /// ([`Browsers::checked`]), sealed with it.
    pub fn create_options_carrying(
        &self,
        proof: Proof<'_>,
        carried: Option<serde_json::Value>,
    ) -> Result<Ceremony, BrowserError> {
        let relying = self.relying()?;
        let now = self.clock.now();
        let (code, exclude) = {
            let state = self.state();
            let code = match proof {
                Proof::Code(code) => Some(live_setup(&state, code, now)?),
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
            .start_passkey_registration(relying.handle, "glave", "glave", Some(exclude))
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let registration: PasskeyRegistration = stripped(&registration, "/rs/exclude_credentials")?;
        let mut request = Request::new(Kind::Make, relying.handle, now, registration)?;
        request.code = code;
        request.carried = carried;
        let mut asked = ceremony(self.sealer.seal(&request)?, &options)?;
        if matches!(relying.party, Party::Public) {
            discoverable(&mut asked.options);
        }
        Ok(asked)
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
        let (passkey, change, _) = self.made(ceremony, credential, name, Proved::Code)?;
        let id = parse(&passkey.id).ok_or(BrowserError::Ceremony)?;
        let session = self.begin(Some(id), vec![change]).await?;
        Ok((passkey, session))
    }

    /// The passkey `credential` answers the request `ceremony` with, called `name` if named,
    /// checked but not kept yet, the request one [`Browsers::create_options_carrying`] began
    /// carrying something: the first of a new glave account, which [`Browsers::keep`] keeps
    /// once the invite or link it was made by is spent, which spends the request too.
    pub fn checked(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<Checked, BrowserError> {
        let (info, change, _) = self.made(ceremony, credential, name, Proved::Carried)?;
        Ok(Checked { info, change })
    }

    /// Keeps the passkey `checked`, made from a link sent to the account's email, and signs
    /// this browser in with it: one the person's other devices are told of.
    pub async fn keep_emailed(&self, checked: Checked) -> Result<Session, BrowserError> {
        let Checked { mut info, change } = checked;
        info.emailed = true;
        // A passkey checked is always one to add.
        let Change::PasskeyAdded(id, name, kept, party, _) = change else {
            return Err(BrowserError::Ceremony);
        };
        let change = Change::PasskeyAdded(id, name, kept, party, true);
        self.keep(Checked { info, change }).await
    }

    /// Keeps the passkey `checked`, and signs this browser in with it.
    pub async fn keep(&self, checked: Checked) -> Result<Session, BrowserError> {
        let id = parse(&checked.info.id).ok_or(BrowserError::Ceremony)?;
        self.begin(Some(id), vec![checked.change]).await
    }

    /// Makes the passkey `credential` answers the request `ceremony` with, called `name` if
    /// named, where the person signed in made the request.
    pub async fn add(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
    ) -> Result<PasskeyInfo, BrowserError> {
        let (passkey, change, _) = self.made(ceremony, credential, name, Proved::SignedIn)?;
        self.change(|_, _| vec![change.clone()]).await?;
        Ok(passkey)
    }

    /// The passkey `credential` answers the request `ceremony` with, called `name` if named,
    /// its record, and what the request carried: a request `proved` as it says. One a setup
    /// code proved spends the code; one the person signed in made is spent here; one that
    /// carries something is spent with what proved it, by the caller.
    fn made(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
        name: Option<&str>,
        proved: Proved,
    ) -> Result<(PasskeyInfo, Change, Option<serde_json::Value>), BrowserError> {
        let relying = self.relying()?;
        let now = self.clock.now();
        let making: Request<PasskeyRegistration> = self.sealer.open(ceremony, Kind::Make, now)?;
        let as_proved = match proved {
            Proved::Code => making.code.is_some(),
            Proved::SignedIn => making.code.is_none() && making.carried.is_none(),
            Proved::Carried => making.code.is_none() && making.carried.is_some(),
        };
        if making.handle != relying.handle || !as_proved {
            return Err(BrowserError::Ceremony);
        }
        let answer: RegisterPublicKeyCredential = serde_json::from_value(credential.clone())
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let passkey = relying
            .webauthn
            .finish_passkey_registration(&answer, &making.state)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        if making.carried.is_none() {
            self.sealer
                .spend(relying.handle, (making.id, making.began), now)?;
        }
        // The passkeys the request named for the browser to make none again, as kept now.
        let again = self
            .state()
            .passkeys
            .iter()
            .filter(|kept| relying.made(kept))
            .any(|kept| kept.credential.cred_id() == passkey.cred_id());
        if again {
            let excluded = WebauthnError::CredentialExcludedFromRequest;
            return Err(BrowserError::Passkey(excluded.to_string()));
        }
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
            emailed: false,
        };
        let party = relying.id.clone();
        let change = Change::PasskeyAdded(id, name, kept, party, false);
        Ok((info, change, making.carried))
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
        let state: DiscoverableAuthentication = stripped(&authentication, "/ast/credentials")?;
        let request = Request::new(Kind::Use, relying.handle, self.clock.now(), state)?;
        ceremony(self.sealer.seal(&request)?, &options)
    }

    /// Signs a browser in with the passkey `credential` answers the request `ceremony` with:
    /// one of the person's in the store as it is now, else [`BrowserError::Unknown`] when it
    /// carries the person's handle, so the browser can tell its passkey manager.
    pub async fn get(
        &self,
        ceremony: &str,
        credential: &serde_json::Value,
    ) -> Result<Session, BrowserError> {
        let relying = self.relying()?;
        let using: Request<DiscoverableAuthentication> =
            self.sealer.open(ceremony, Kind::Use, self.clock.now())?;
        if using.handle != relying.handle {
            return Err(BrowserError::Ceremony);
        }
        let answer: PublicKeyCredential = serde_json::from_value(credential.clone())
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        // Removed, or added, on another server on this store since this one last looked.
        self.catch_up().await?;
        let (id, kept) = self.kept(relying, &answer)?;
        // Checked against the passkey it answered with, as it is kept now.
        let key = DiscoverableKey::from(&kept);
        let result = relying
            .webauthn
            .finish_discoverable_authentication(&answer, using.state, &[key])
            .map_err(|error| BrowserError::Passkey(error.to_string()))?;
        let spent = (using.id, using.began);
        self.sealer.spend(relying.handle, spent, self.clock.now())?;
        self.used(id, kept, &result).await
    }

    /// Signs a browser in with the passkey `id`, `kept` as it was before `result` of using it.
    async fn used(
        &self,
        id: Uuid,
        mut kept: Passkey,
        result: &AuthenticationResult,
    ) -> Result<Session, BrowserError> {
        let changed = (kept.update_credential(result) == Some(true))
            .then(|| serde_json::to_string(&kept))
            .transpose()
            .map_err(|error| BrowserError::Keep(error.to_string()))?;
        self.begin(Some(id), vec![Change::PasskeyUsed(id, changed)])
            .await
    }

    /// The passkey `answer` signs in with, of those made for `relying`: its id and credential.
    fn kept(
        &self,
        relying: &Relying,
        answer: &PublicKeyCredential,
    ) -> Result<(Uuid, Passkey), BrowserError> {
        let presented = answer.get_credential_id();
        self.state()
            .passkeys
            .iter()
            .filter(|kept| relying.made(kept))
            .find(|kept| kept.credential.cred_id().as_slice() == presented)
            .map(|kept| (kept.id, kept.credential.clone()))
            .ok_or_else(|| relying.unknown(answer))
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
                emailed: kept.emailed,
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
}

/// What proved a request to make a passkey, as its answer says.
#[derive(Clone, Copy)]
enum Proved {
    /// A setup code.
    Code,
    /// The person, signed in.
    SignedIn,
    /// What the request carries, which its caller checks.
    Carried,
}

/// A request's `state` without the passkeys it names at `named`, so the request the browser
/// holds stays small however many passkeys the person has, up to 32 with ids of up to 1,023
/// bytes each. Its answer is checked against the person's passkeys as they are kept then: a
/// sign-in's against the one it answered with, as a request for any passkey's is (the state
/// it becomes); a new passkey's against those made already, which the request named for the
/// browser to make none again.
fn stripped<T: serde::de::DeserializeOwned>(
    state: &impl serde::Serialize,
    named: &str,
) -> Result<T, BrowserError> {
    let refused = |error: serde_json::Error| BrowserError::Passkey(error.to_string());
    let mut state = serde_json::to_value(state).map_err(refused)?;
    let passkeys = state.pointer_mut(named).ok_or_else(|| {
        BrowserError::Passkey(format!("a passkey request's state has no {named}"))
    })?;
    *passkeys = serde_json::Value::Array(Vec::new());
    serde_json::from_value(state).map_err(refused)
}

/// The digest of `code`, when it is a live setup code.
fn live_setup(state: &State, code: &str, now: Timestamp) -> Result<[u8; 32], BrowserError> {
    let presented = digest(code);
    let live = state
        .setups
        .iter()
        .any(|(kept, made)| same(kept, &presented) && now.duration_since(*made) <= SETUP_LIFETIME);
    live.then_some(presented).ok_or(BrowserError::Setup)
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

/// The request the browser holds as `id`, its options as JSON.
fn ceremony(id: String, options: &impl serde::Serialize) -> Result<Ceremony, BrowserError> {
    Ok(Ceremony {
        id,
        options: serde_json::to_value(options)
            .map_err(|error| BrowserError::Passkey(error.to_string()))?,
    })
}

/// `options` asking for a passkey a request naming none finds (a discoverable credential), as
/// a server with glave accounts asks: password managers' passkeys always are, a security key's
/// only when asked.
fn discoverable(options: &mut serde_json::Value) {
    if let Some(selection) = options
        .pointer_mut("/publicKey/authenticatorSelection")
        .and_then(serde_json::Value::as_object_mut)
    {
        selection.insert("residentKey".into(), "required".into());
        selection.insert("requireResidentKey".into(), true.into());
    }
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
