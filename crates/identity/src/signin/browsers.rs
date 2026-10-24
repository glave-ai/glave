//! Browser sessions on the person's own server (`specs/one-server.md`, "Finding it"). A
//! program on this computer signed in as the local person asks for a link; its code, good once
//! and for five minutes, is traded for a browser session: a bearer token the browser keeps in
//! its origin's storage, for every tab, and sends as a header, never a cookie, good for 30
//! days from its last use. Passkeys sign browsers in too (`passkeys`): at the server's public
//! origin where it is reached from elsewhere, else on this computer at `localhost`, each with
//! the relying party it was made for alone. Codes are kept in memory, as their SHA-256 alone.
//! Sessions and passkeys are kept in the store, in the browsers ledger of the person's
//! personal project (glave's own stream under a reserved id), each session as the SHA-256 of
//! its token alone, so nothing kept signs anyone in; every server on one store signs in the
//! same browsers. The server's id, which a browser checks before it presents its session, and
//! the person's passkey handle, which must outlive the store, are the caller's to keep.

mod ledger;
mod passkeys;
mod state;

pub use self::passkeys::{Accepted, Ceremony, PasskeyInfo, Proof};

use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use glave_event::{RawEvent, Secret, Seq};
use glave_ports::host::PublicOrigin;
use glave_ports::stream::{Reserved, StreamError};
use glave_ports::{BoxFuture, Clock, EventLog, Scope};
use jiff::{SignedDuration, Timestamp};
use thiserror::Error;
use uuid::Uuid;

use self::ledger::{Ledger, Made, Stream};
use self::passkeys::{Ceremonies, Relying};
use self::state::{Change, Kept, State};
use super::local::{digest, hex, same};

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

/// Why a browser could not sign in, or its sessions could not be kept.
#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("that sign-in link was used already or has expired: open glave again for a new one")]
    Code,
    #[error(
        "that link to make a passkey was used already or has expired: on the computer glave \
         runs on, ask glave for a new one"
    )]
    Setup,
    #[error("too many sign-in links that do not work were tried: wait a minute, then try again")]
    TooMany,
    #[error("this browser's sign-in has ended: open glave again to sign in")]
    Ended,
    #[error("this browser was signed in on the computer glave runs on, and signs in only there")]
    Elsewhere,
    #[error("this glave makes no passkeys for this page: open glave on its computer to sign in")]
    NoPasskeys,
    #[error("no passkey signs in to this glave yet: open a link glave gives you, and make one")]
    NoPasskey,
    #[error("that passkey request has expired: try again")]
    Ceremony,
    /// A passkey with the person's handle that this server does not know, removed or never
    /// made here: the relying party's id, and its credential id in base64url, for the browser
    /// to tell its passkey manager.
    #[error(
        "that passkey no longer signs in to this glave: sign in with another, or open a link \
         glave gives you to make a new one"
    )]
    Unknown { party: String, credential: String },
    #[error("the passkey was not accepted: {0}")]
    Passkey(String),
    #[error("could not keep this computer's browser sign-ins: {0}")]
    Keep(String),
    #[error("could not get random bytes for a sign-in: {0}")]
    Random(String),
}

impl From<StreamError> for BrowserError {
    fn from(error: StreamError) -> Self {
        Self::Keep(error.to_string())
    }
}

/// A session just begun: the token the browser keeps, the id that names the session in
/// records and when it is ended, and the passkeys that sign the person in, for the browser's
/// passkey manager, where the server makes passkeys for that browser's page.
#[derive(Debug)]
pub struct Session {
    pub token: Secret,
    pub id: String,
    pub accepted: Option<Accepted>,
}

/// The codes, browser sessions and passkeys of one server, the sessions and passkeys kept in
/// the store.
pub struct Browsers {
    ledger: Box<dyn Ledger>,
    server: String,
    /// The person's user handle, which every passkey made for them keeps.
    handle: Uuid,
    clock: Box<dyn Clock>,
    state: Mutex<State>,
    /// Passkeys' relying party: the public origin, where the server is reached from
    /// elsewhere; else this computer's `localhost`, once the server listens there.
    relying: OnceLock<Relying>,
    /// The passkey requests begun and not yet answered.
    ceremonies: Mutex<Ceremonies>,
}

impl Browsers {
    /// The sessions and passkeys the browsers ledger in `scope` of `log` holds, for the server
    /// whose id is `server`, reached from elsewhere at `origin` if anywhere: there it makes
    /// passkeys and signs browsers in with them; with none, on this computer once it listens
    /// here ([`Browsers::reached_here`]). Every passkey it makes keeps `handle`, the person's.
    pub async fn open<L: EventLog + 'static>(
        log: L,
        scope: Scope,
        server: String,
        handle: Uuid,
        origin: Option<&PublicOrigin>,
        clock: Box<dyn Clock>,
    ) -> Result<Self, BrowserError> {
        let thread = Reserved::Browsers.id();
        let relying = match origin {
            Some(origin) => OnceLock::from(Relying::public(origin, handle)?),
            None => OnceLock::new(),
        };
        let browsers = Self {
            ledger: Box::new(Stream { log, scope, thread }),
            server,
            handle,
            clock,
            state: Mutex::new(State {
                next: 1,
                ..State::default()
            }),
            relying,
            ceremonies: Mutex::new(Ceremonies::default()),
        };
        browsers.catch_up().await?;
        Ok(browsers)
    }

    /// The server's id, which `server/discover` answers so a browser presents its session to
    /// no other.
    pub fn server(&self) -> String {
        self.server.clone()
    }

    /// A new code for a link, good once, for `CODE_LIFETIME`.
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
        self.begin(None, Vec::new()).await
    }

    /// Begins a session, signed in with `passkey` if with one, recording `with` beside it.
    async fn begin(
        &self,
        passkey: Option<Uuid>,
        with: Vec<Change>,
    ) -> Result<Session, BrowserError> {
        let now = self.clock.now();
        let token = format!("{SESSION_PREFIX}{}", random_hex()?);
        let id = new_id(now)?;
        let hash = hex(&digest(&token));
        let with = &with;
        self.change(|state, now| {
            let mut changes = with.clone();
            let live: Vec<&Kept> = state.live(now).collect();
            if live.len() >= MOST_SESSIONS
                && let Some(oldest) = live.iter().min_by_key(|session| session.used)
            {
                changes.push(Change::SignedOut(vec![oldest.id]));
            }
            changes.push(Change::SignedIn(id, hash.clone(), passkey));
            changes
        })
        .await?;
        Ok(Session {
            token: Secret::new(token),
            id: id.to_string(),
            accepted: self.accepted(passkey.is_some()),
        })
    }

    /// The id of the session `token` is, renewed for `SESSION_LIFETIME` from now, when it is
    /// presented `here`, on the server's own computer, or was signed in with a passkey.
    pub async fn verify(&self, token: &str, here: bool) -> Result<String, BrowserError> {
        let presented = hex(&digest(token));
        let mut found = self.find(&presented);
        if found.is_none() {
            // Signed in on another server on this store since this one last looked.
            self.catch_up().await?;
            found = self.find(&presented);
        }
        let (id, used, public) = found.ok_or(BrowserError::Ended)?;
        // Checked before it is renewed, so a session begun on this computer, by a link or a
        // passkey made here, is never renewed from elsewhere.
        if !here && !public {
            return Err(BrowserError::Elsewhere);
        }
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

    /// The live session whose token's hash is `presented`: its id, when it was last used,
    /// and whether a passkey made at a public origin signed it in, so it signs in through
    /// that origin too.
    fn find(&self, presented: &str) -> Option<(Uuid, Timestamp, bool)> {
        let now = self.clock.now();
        let state = self.state();
        let session = state
            .live(now)
            .find(|session| same(session.hash.as_bytes(), presented.as_bytes()))?;
        let public = session.passkey.is_some_and(|passkey| {
            state
                .passkeys
                .iter()
                .any(|kept| kept.id == passkey && kept.public())
        });
        Some((session.id, session.used, public))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends what `make` makes of the sessions known, reading what other servers wrote and
    /// trying again when one wrote first; nothing when it makes nothing.
    async fn change(
        &self,
        make: impl Fn(&State, Timestamp) -> Vec<Change> + Sync,
    ) -> Result<(), BrowserError> {
        let (this, make) = (self, &make);
        let mut attempt = move || -> BoxFuture<'_, Made> {
            Box::pin(async move {
                this.catch_up().await?;
                let state = this.state();
                let now = this.clock.now();
                let changes = make(&state, now);
                if changes.is_empty() {
                    return Ok(None);
                }
                let mut events = Vec::with_capacity(changes.len());
                for (n, change) in changes.into_iter().enumerate() {
                    let seq = state
                        .next
                        .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
                    events.push(change.event(seq, now)?);
                }
                Ok(Some((events, ())))
            })
        };
        let appended = self.ledger.append_retrying(&mut attempt).await?;
        let mut state = self.state();
        for raw in appended.into_iter().flat_map(|(events, ())| events) {
            state.take(raw);
        }
        Ok(())
    }

    /// Reads what the ledger holds past what is known.
    async fn catch_up(&self) -> Result<(), BrowserError> {
        let after = Seq::new(self.state().next.saturating_sub(1)).ok();
        let mut take = |page: Vec<RawEvent>| {
            let mut state = self.state();
            for raw in page {
                state.take(raw);
            }
        };
        Ok(self.ledger.catch_up(after, &mut take).await?)
    }
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
