//! Browser sessions on the person's own server (`specs/one-server.md`, "Finding it"). A
//! program on this computer signed in as the local person asks for a link; its code, good once
//! and for five minutes, is traded for a browser session: a bearer token the tab keeps and
//! sends as a header, never a cookie, accepted only from loopback as the local person, and
//! good for 30 days from its last use. Codes are kept in memory, as their SHA-256 alone.
//! Sessions are kept in the store, in the browsers ledger of the person's personal project
//! (glave's own stream under a reserved id), as the SHA-256 of each token alone, so nothing
//! kept signs anyone in; every server on one store signs in the same browsers. The server's
//! id, which a browser checks before it presents its session, is the caller's to keep.

mod earlier;
mod ledger;

use std::sync::{Mutex, MutexGuard, PoisonError};

use glave_event::{
    At, Body, BrowserSignedIn, BrowserSignedOut, BrowserUsed, EventId, Header, RawEvent, Secret,
    Seq, ThreadId, decode, encode,
};
use glave_ports::{Clock, EventLog, LogError, Scope, reserved_ledger};
use jiff::{SignedDuration, Timestamp};
use thiserror::Error;
use uuid::Uuid;

use self::ledger::{Ledger, Stream};
use super::local::{digest, hex, same};

pub use self::earlier::Earlier;

/// What every browser session's token starts with.
pub(crate) const SESSION_PREFIX: &str = "glave_session_";

/// How long a link's code may wait to be traded for a session.
pub(crate) const CODE_LIFETIME: SignedDuration = SignedDuration::from_mins(5);

/// How long a session lasts after it was last used.
pub(crate) const SESSION_LIFETIME: SignedDuration = SignedDuration::from_hours(30 * 24);

/// How stale a session's last use may be before a use is recorded again, so a busy tab does
/// not write at every request and the ledger grows by a few events a day at most.
const RECORD_USE: SignedDuration = SignedDuration::from_hours(24);

/// Codes waiting at once; a new one past this forgets the oldest.
const MOST_CODES: usize = 16;

/// Sessions kept at once; a new one past this ends the one used longest ago.
const MOST_SESSIONS: usize = 64;

/// Random bytes in a code or a session's token.
const SECRET_BYTES: usize = 32;

/// Appends tried before giving up on a ledger other servers keep moving.
const ATTEMPTS: usize = 8;

/// The browsers ledger's reserved id, `00000000-0000-7000-8000-000000000005`: the browser
/// sessions of the person on their own server, in their personal project.
pub fn browsers_ledger() -> Option<ThreadId> {
    reserved_ledger(5)
}

/// Why a browser could not sign in, or its sessions could not be kept.
#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("that sign-in link was used already or has expired: open glave again for a new one")]
    Code,
    #[error("this browser's sign-in has ended: open glave again to sign in")]
    Ended,
    #[error("could not keep this computer's browser sign-ins: {0}")]
    Keep(String),
    #[error("could not get random bytes for a sign-in: {0}")]
    Random(String),
}

/// A session just begun: the token the browser keeps, and the id that names the session in
/// records and when it is ended.
#[derive(Debug)]
pub struct Session {
    pub token: Secret,
    pub id: String,
}

/// The codes and browser sessions of one server, the sessions kept in the store.
pub struct Browsers {
    ledger: Box<dyn Ledger>,
    server: String,
    clock: Box<dyn Clock>,
    state: Mutex<State>,
}

/// What is known: the codes waiting, and the sessions as the ledger has them so far.
#[derive(Default)]
struct State {
    /// Each code's digest and when it was made.
    codes: Vec<([u8; 32], Timestamp)>,
    sessions: Vec<Kept>,
    /// The `seq` the ledger's next event takes.
    next: u64,
}

/// One session as the ledger has it.
struct Kept {
    id: Uuid,
    /// The SHA-256 of its token, in hex.
    hash: String,
    /// When it was last recorded as used.
    used: Timestamp,
}

impl Browsers {
    /// The sessions the browsers ledger in `scope` of `log` holds, for the server whose id is
    /// `server`.
    pub async fn open<L: EventLog + 'static>(
        log: L,
        scope: Scope,
        server: String,
        clock: Box<dyn Clock>,
    ) -> Result<Self, BrowserError> {
        let thread = ledger_id()?;
        let browsers = Self {
            ledger: Box::new(Stream { log, scope, thread }),
            server,
            clock,
            state: Mutex::new(State {
                next: 1,
                ..State::default()
            }),
        };
        browsers.catch_up().await?;
        Ok(browsers)
    }

    /// The server's id, which `server/discover` answers so a browser presents its session to
    /// no other.
    pub fn server(&self) -> String {
        self.server.clone()
    }

    /// A new code for a link, good once, for [`CODE_LIFETIME`].
    pub fn link(&self) -> Result<Secret, BrowserError> {
        let code = random_hex()?;
        let now = self.clock.now();
        let mut state = self.state();
        state
            .codes
            .retain(|(_, made)| now.duration_since(*made) <= CODE_LIFETIME);
        if state.codes.len() >= MOST_CODES {
            state.codes.remove(0);
        }
        state.codes.push((digest(&code), now));
        Ok(Secret::new(code))
    }

    /// Trades `code` for a new session; the code is spent whether or not it is kept.
    pub async fn redeem(&self, code: &str) -> Result<Session, BrowserError> {
        let now = self.clock.now();
        {
            let mut state = self.state();
            let presented = digest(code);
            let found = state.codes.iter().position(|(kept, made)| {
                same(kept, &presented) && now.duration_since(*made) <= CODE_LIFETIME
            });
            let Some(at) = found else {
                return Err(BrowserError::Code);
            };
            state.codes.remove(at);
        }
        let token = format!("{SESSION_PREFIX}{}", random_hex()?);
        let id = new_id(now)?;
        let hash = hex(&digest(&token));
        self.change(|state, now| {
            let mut changes = Vec::new();
            let live: Vec<&Kept> = state.live(now).collect();
            if live.len() >= MOST_SESSIONS
                && let Some(oldest) = live.iter().min_by_key(|session| session.used)
            {
                changes.push(Change::SignedOut(vec![oldest.id]));
            }
            changes.push(Change::SignedIn(id, hash.clone()));
            changes
        })
        .await?;
        Ok(Session {
            token: Secret::new(token),
            id: id.to_string(),
        })
    }

    /// The id of the session `token` is, renewed for [`SESSION_LIFETIME`] from now.
    pub async fn verify(&self, token: &str) -> Result<String, BrowserError> {
        let presented = hex(&digest(token));
        let mut found = self.find(&presented);
        if found.is_none() {
            // Signed in on another server on this store since this one last looked.
            self.catch_up().await?;
            found = self.find(&presented);
        }
        let (id, used) = found.ok_or(BrowserError::Ended)?;
        if self.clock.now().duration_since(used) >= RECORD_USE {
            // A use not recorded is recorded with the next one: the session goes on.
            let _ = self.change(|_, _| vec![Change::Used(id)]).await;
        }
        Ok(id.to_string())
    }

    /// Ends the session `id`.
    pub async fn end(&self, id: &str) -> Result<(), BrowserError> {
        let Ok(id) = Uuid::parse_str(id) else {
            return Ok(());
        };
        self.change(|state, _| {
            let kept = state.sessions.iter().any(|session| session.id == id);
            kept.then(|| Change::SignedOut(vec![id]))
                .into_iter()
                .collect()
        })
        .await
    }

    /// Ends every session but `keep`'s: "Sign out other browsers".
    pub async fn end_others(&self, keep: Option<&str>) -> Result<(), BrowserError> {
        let keep = keep.and_then(|id| Uuid::parse_str(id).ok());
        self.change(|state, now| {
            let others: Vec<Uuid> = state
                .live(now)
                .map(|session| session.id)
                .filter(|id| Some(*id) != keep)
                .collect();
            (!others.is_empty())
                .then_some(Change::SignedOut(others))
                .into_iter()
                .collect()
        })
        .await
    }

    /// Keeps the sessions an earlier glave kept in a file, unless the ledger knows them
    /// already, as used now; those whose lifetime ended are left out.
    pub async fn take_in(&self, earlier: &Earlier) -> Result<(), BrowserError> {
        self.change(|state, now| {
            earlier
                .live(now)
                .filter(|(id, _)| !state.sessions.iter().any(|kept| kept.id == *id))
                .map(|(id, hash)| Change::SignedIn(id, hash.to_owned()))
                .collect()
        })
        .await
    }

    /// The live session whose token's hash is `presented`: its id, and when it was last used.
    fn find(&self, presented: &str) -> Option<(Uuid, Timestamp)> {
        let now = self.clock.now();
        let state = self.state();
        state
            .live(now)
            .find(|session| same(session.hash.as_bytes(), presented.as_bytes()))
            .map(|session| (session.id, session.used))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends what `make` makes of the sessions known, reading what other servers wrote and
    /// trying again when one wrote first; nothing when it makes nothing.
    async fn change(
        &self,
        make: impl Fn(&State, Timestamp) -> Vec<Change>,
    ) -> Result<(), BrowserError> {
        for _ in 0..ATTEMPTS {
            let events = {
                let state = self.state();
                let now = self.clock.now();
                let changes = make(&state, now);
                if changes.is_empty() {
                    return Ok(());
                }
                let mut events = Vec::with_capacity(changes.len());
                for (n, change) in changes.into_iter().enumerate() {
                    let seq = state
                        .next
                        .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
                    events.push(change.event(seq, now)?);
                }
                events
            };
            match self.ledger.append(&events).await {
                Ok(()) => {
                    let mut state = self.state();
                    for raw in events {
                        state.take(raw);
                    }
                    return Ok(());
                }
                Err(LogError::Conflict { .. }) => self.catch_up().await?,
                Err(error) => return Err(BrowserError::Keep(error.to_string())),
            }
        }
        Err(BrowserError::Keep(
            "they kept changing while this server wrote to them".into(),
        ))
    }

    /// Reads what the ledger holds past what is known.
    async fn catch_up(&self) -> Result<(), BrowserError> {
        loop {
            let after = {
                let next = self.state().next;
                Seq::new(next.saturating_sub(1)).ok()
            };
            let page = self
                .ledger
                .read(after)
                .await
                .map_err(|error| BrowserError::Keep(error.to_string()))?;
            let full = page.len() >= ledger::PAGE;
            let mut state = self.state();
            for raw in page {
                state.take(raw);
            }
            if !full {
                return Ok(());
            }
        }
    }
}

impl State {
    /// The sessions used within [`SESSION_LIFETIME`] of `now`.
    fn live(&self, now: Timestamp) -> impl Iterator<Item = &Kept> {
        self.sessions
            .iter()
            .filter(move |session| now.duration_since(session.used) <= SESSION_LIFETIME)
    }

    /// Takes in the ledger's event at the next `seq`; any other is already in, or not yet.
    fn take(&mut self, raw: RawEvent) {
        if raw.seq.get() != self.next {
            return;
        }
        self.next = self.next.saturating_add(1);
        let at = raw.at.timestamp();
        let Ok(event) = decode(raw) else {
            return;
        };
        match event.body {
            Body::BrowserSignedIn(signed_in) => self.sessions.push(Kept {
                id: signed_in.session,
                hash: signed_in.hash,
                used: at,
            }),
            Body::BrowserUsed(used) => {
                for session in &mut self.sessions {
                    if session.id == used.session {
                        session.used = session.used.max(at);
                    }
                }
            }
            Body::BrowserSignedOut(out) => self
                .sessions
                .retain(|session| !out.sessions.contains(&session.id)),
            _ => {}
        }
    }
}

/// One change to the sessions.
enum Change {
    /// A session began: its id, and its token's hash.
    SignedIn(Uuid, String),
    Used(Uuid),
    SignedOut(Vec<Uuid>),
}

impl Change {
    /// This change as the ledger's event at `seq`, made at `now`.
    fn event(self, seq: u64, now: Timestamp) -> Result<RawEvent, BrowserError> {
        let failed = |error: &dyn std::fmt::Display| BrowserError::Keep(error.to_string());
        let header = Header {
            id: EventId::try_from(new_id(now)?).map_err(|e| failed(&e))?,
            thread: ledger_id()?,
            seq: Seq::new(seq).map_err(|e| failed(&e))?,
            at: At::from(now),
        };
        match self {
            Self::SignedIn(session, hash) => encode(header, &BrowserSignedIn { session, hash }),
            Self::Used(session) => encode(header, &BrowserUsed { session }),
            Self::SignedOut(sessions) => encode(header, &BrowserSignedOut { sessions }),
        }
        .map_err(|e| failed(&e))
    }
}

/// The ledger's reserved id.
fn ledger_id() -> Result<ThreadId, BrowserError> {
    browsers_ledger().ok_or_else(|| BrowserError::Keep("the browsers ledger has no id".into()))
}

/// A UUID v7 at `now`.
fn new_id(now: Timestamp) -> Result<Uuid, BrowserError> {
    let mut random = [0u8; 10];
    getrandom::fill(&mut random).map_err(|e| BrowserError::Random(e.to_string()))?;
    let millis = u64::try_from(now.as_millisecond()).unwrap_or_default();
    Ok(uuid::Builder::from_unix_timestamp_millis(millis, &random).into_uuid())
}

/// [`SECRET_BYTES`] random bytes from the operating system, in hex.
fn random_hex() -> Result<String, BrowserError> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| BrowserError::Random(e.to_string()))?;
    Ok(hex(&bytes))
}

#[cfg(test)]
mod tests;
