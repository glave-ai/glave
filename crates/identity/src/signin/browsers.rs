//! Browser sessions on the person's own server (`specs/one-server.md`, "Finding it"). A
//! program on this computer signed in as the local person asks for a link; its code, good once
//! and for five minutes, is traded for a browser session: a bearer token the tab keeps and
//! sends as a header, never a cookie, accepted only from loopback as the local person, and
//! good for 30 days from its last use. Codes are kept in memory and sessions in a file only the
//! person can read, both as their SHA-256 alone, so neither holds anything that signs in. The
//! file also keeps the server's id, which a browser checks before it presents its session.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use glave_event::Secret;
use glave_ports::Clock;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::local::{digest, hex, keep, same};

/// What every browser session's token starts with.
pub(crate) const SESSION_PREFIX: &str = "glave_session_";

/// How long a link's code may wait to be traded for a session.
pub(crate) const CODE_LIFETIME: SignedDuration = SignedDuration::from_mins(5);

/// How long a session lasts after it was last used.
pub(crate) const SESSION_LIFETIME: SignedDuration = SignedDuration::from_hours(30 * 24);

/// How stale a session's last use may be before a use is written down again, so a busy tab
/// does not write the file at every request.
const RECORD_USE: SignedDuration = SignedDuration::from_hours(1);

/// Codes waiting at once; a new one past this forgets the oldest.
const MOST_CODES: usize = 16;

/// Sessions kept at once; a new one past this ends the one used longest ago.
const MOST_SESSIONS: usize = 64;

/// The version of the file's format.
const VERSION: u32 = 1;

/// Random bytes in a code or a session's token.
const SECRET_BYTES: usize = 32;

/// Why a browser could not sign in, or its sessions could not be kept.
#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("that sign-in link was used already or has expired: open glave again for a new one")]
    Code,
    #[error("this browser's sign-in has ended: open glave again to sign in")]
    Ended,
    #[error("could not keep this computer's browser sign-ins in {path}: {reason}")]
    Keep { path: String, reason: String },
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

/// The codes and browser sessions of one server, the sessions kept in a file.
pub struct Browsers {
    path: PathBuf,
    clock: Box<dyn Clock>,
    state: Mutex<State>,
    /// The latest generation written to the file, so an older write never lands last.
    written: Arc<Mutex<u64>>,
}

/// What is kept in memory: the codes waiting, and the file's contents.
struct State {
    /// Each code's digest and when it was made.
    codes: Vec<([u8; 32], Timestamp)>,
    kept: Kept,
    /// Counts changes to `kept`.
    generation: u64,
}

/// The file.
#[derive(Serialize, Deserialize)]
struct Kept {
    version: u32,
    /// The server's id, the same across restarts.
    server: String,
    sessions: Vec<KeptSession>,
}

#[derive(Serialize, Deserialize)]
struct KeptSession {
    id: String,
    /// The SHA-256 of the token, in hex.
    hash: String,
    /// When it was last used, to the hour.
    used: Timestamp,
}

impl Browsers {
    /// The sessions kept at `path`, and the server's id there; a new id when the file is
    /// missing or unreadable, which ends every session it held.
    pub fn open(path: PathBuf, clock: Box<dyn Clock>) -> Result<Self, BrowserError> {
        let read = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Kept>(&text)
                .ok()
                .filter(|kept| kept.version == VERSION),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(kept_error(&path, &e)),
        };
        let kept = if let Some(kept) = read {
            kept
        } else {
            let fresh = Kept {
                version: VERSION,
                server: new_id(clock.now())?,
                sessions: Vec::new(),
            };
            keep(&path, &contents(&fresh)).map_err(|e| kept_error(&path, &e))?;
            fresh
        };
        Ok(Self {
            path,
            clock,
            state: Mutex::new(State {
                codes: Vec::new(),
                kept,
                generation: 0,
            }),
            written: Arc::new(Mutex::new(0)),
        })
    }

    /// The server's id, which `server/discover` answers so a browser presents its session to
    /// no other.
    pub fn server(&self) -> String {
        self.state().kept.server.clone()
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
        let token = format!("{SESSION_PREFIX}{}", random_hex()?);
        let now = self.clock.now();
        let id = new_id(now)?;
        let written = {
            let mut state = self.state();
            let presented = digest(code);
            let found = state.codes.iter().position(|(kept, made)| {
                same(kept, &presented) && now.duration_since(*made) <= CODE_LIFETIME
            });
            let Some(at) = found else {
                return Err(BrowserError::Code);
            };
            state.codes.remove(at);
            state.forget_ended(now);
            if state.kept.sessions.len() >= MOST_SESSIONS
                && let Some(oldest) = (0..state.kept.sessions.len())
                    .min_by_key(|at| state.kept.sessions.get(*at).map(|session| session.used))
            {
                state.kept.sessions.remove(oldest);
            }
            state.kept.sessions.push(KeptSession {
                id: id.clone(),
                hash: hex(&digest(&token)),
                used: now,
            });
            state.changed()
        };
        self.write(written).await?;
        Ok(Session {
            token: Secret::new(token),
            id,
        })
    }

    /// The id of the session `token` is, renewed for [`SESSION_LIFETIME`] from now.
    pub async fn verify(&self, token: &str) -> Result<String, BrowserError> {
        let now = self.clock.now();
        let presented = hex(&digest(token));
        let (id, written) = {
            let mut state = self.state();
            let forgot = state.forget_ended(now);
            let session = state
                .kept
                .sessions
                .iter_mut()
                .find(|session| same(session.hash.as_bytes(), presented.as_bytes()))
                .ok_or(BrowserError::Ended)?;
            let renew = now.duration_since(session.used) >= RECORD_USE;
            if renew {
                session.used = now;
            }
            let id = session.id.clone();
            (id, (renew || forgot).then(|| state.changed()))
        };
        if let Some(written) = written {
            // A use not written down is written with the next one: the session goes on.
            let _ = self.write(written).await;
        }
        Ok(id)
    }

    /// Ends the session `id`.
    pub async fn end(&self, id: &str) -> Result<(), BrowserError> {
        let written = {
            let mut state = self.state();
            state.kept.sessions.retain(|session| session.id != id);
            state.changed()
        };
        self.write(written).await
    }

    /// Ends every session but `keep`'s: "Sign out other browsers".
    pub async fn end_others(&self, keep: Option<&str>) -> Result<(), BrowserError> {
        let written = {
            let mut state = self.state();
            state
                .kept
                .sessions
                .retain(|session| keep.is_some_and(|id| session.id == id));
            state.changed()
        };
        self.write(written).await
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the file as it was at `generation`, unless a later generation is written already.
    async fn write(&self, (generation, text): (u64, String)) -> Result<(), BrowserError> {
        let path = self.path.clone();
        let written = Arc::clone(&self.written);
        let wrote = tokio::task::spawn_blocking(move || {
            let mut last = written.lock().unwrap_or_else(PoisonError::into_inner);
            if generation <= *last {
                return Ok(());
            }
            keep(&path, &text).map_err(|e| kept_error(&path, &e))?;
            *last = generation;
            Ok(())
        })
        .await;
        wrote.map_err(|e| BrowserError::Keep {
            path: self.path.display().to_string(),
            reason: e.to_string(),
        })?
    }
}

impl State {
    /// Forgets sessions unused for [`SESSION_LIFETIME`]; whether there were any.
    fn forget_ended(&mut self, now: Timestamp) -> bool {
        let before = self.kept.sessions.len();
        self.kept
            .sessions
            .retain(|session| now.duration_since(session.used) <= SESSION_LIFETIME);
        self.kept.sessions.len() != before
    }

    /// The next generation and the file's contents at it.
    fn changed(&mut self) -> (u64, String) {
        self.generation += 1;
        (self.generation, contents(&self.kept))
    }
}

fn contents(kept: &Kept) -> String {
    // A struct of strings and timestamps always serializes.
    serde_json::to_string(kept).unwrap_or_default()
}

fn kept_error(path: &std::path::Path, error: &std::io::Error) -> BrowserError {
    BrowserError::Keep {
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

/// A UUID v7 at `now`, as text.
fn new_id(now: Timestamp) -> Result<String, BrowserError> {
    let mut random = [0u8; 10];
    getrandom::fill(&mut random).map_err(|e| BrowserError::Random(e.to_string()))?;
    let millis = u64::try_from(now.as_millisecond()).unwrap_or_default();
    Ok(uuid::Builder::from_unix_timestamp_millis(millis, &random)
        .into_uuid()
        .to_string())
}

/// [`SECRET_BYTES`] random bytes from the operating system, in hex.
fn random_hex() -> Result<String, BrowserError> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| BrowserError::Random(e.to_string()))?;
    Ok(hex(&bytes))
}

#[cfg(test)]
mod tests;
