//! The token a client of `glave computer serve` presents first: a long-lived one a person
//! sets, or a short-lived one the harness side mints for a worker, which only opens
//! connections until it expires. A minted token travels as a secret and a lifetime, and the
//! side that reads it counts the lifetime on its own clock, so two clocks that disagree never
//! shorten it.

use std::fmt::{self, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use glave_ports::computer::ComputerError;

/// Random bytes in a minted token.
const MINTED_BYTES: usize = 32;
/// The shortest secret a listener accepts.
pub const MIN_SECRET: usize = 16;

/// A secret, and when it stops opening connections. A connection opened before then lasts.
#[derive(Clone, PartialEq, Eq)]
pub struct Token {
    secret: String,
    /// How long it opens connections; `None` for a token that does not expire.
    lifetime: Option<Duration>,
    /// When it stops, on the clock of whoever holds it.
    expires: Option<SystemTime>,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("secret", &"<hidden>")
            .field("expires", &self.expires)
            .field("lifetime", &self.lifetime)
            .finish()
    }
}

impl Token {
    /// A token that never expires, such as one a person sets in `GLAVE_COMPUTER_TOKEN`.
    pub fn lasting(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            lifetime: None,
            expires: None,
        }
    }

    /// A new token of 32 random bytes (64 hex digits) that opens connections for `lifetime`
    /// after `now`.
    pub fn mint(now: SystemTime, lifetime: Duration) -> Result<Self, ComputerError> {
        let mut random = [0u8; MINTED_BYTES];
        getrandom::fill(&mut random).map_err(|error| {
            ComputerError::Failed(format!("could not get random bytes for a token: {error}"))
        })?;
        let secret = random.iter().fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
        Ok(Self {
            secret,
            lifetime: Some(lifetime),
            expires: now.checked_add(lifetime),
        })
    }

    /// What a client presents.
    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// When it stops opening connections, if it does.
    pub fn expires(&self) -> Option<SystemTime> {
        self.expires
    }

    /// Whether `given` opens a connection at `now`: the same secret, compared in time that
    /// does not depend on where they differ, before it expires.
    pub fn admits(&self, given: &str, now: SystemTime) -> bool {
        let fresh = self.expires.is_none_or(|expires| now < expires);
        same(given, &self.secret) && fresh
    }

    /// The token as a file holds it: the secret, then how many seconds it lasts, each on its
    /// own line. A clock time would mean the minter's clock, which the reader's may not
    /// match.
    pub fn to_file(&self) -> String {
        let lifetime = self
            .lifetime
            .map_or_else(String::new, |lifetime| lifetime.as_secs().to_string());
        format!("{}\n{lifetime}\n", self.secret)
    }

    /// A token read from a file [`Token::to_file`] wrote, at `now` on the reader's clock: it
    /// lasts its lifetime from then. Refused when the secret is too short to guard a
    /// listener.
    pub fn from_file(text: &str, now: SystemTime) -> Result<Self, ComputerError> {
        let mut lines = text.lines();
        let secret = lines.next().unwrap_or_default().trim().to_owned();
        if secret.len() < MIN_SECRET {
            return Err(ComputerError::Refused(
                format!("the token is shorter than {MIN_SECRET} characters").into(),
            ));
        }
        let lifetime = match lines.next().map(str::trim).filter(|line| !line.is_empty()) {
            None => None,
            Some(seconds) => Some(Duration::from_secs(seconds.parse().map_err(|_| {
                ComputerError::Refused(
                    format!("{seconds:?} is not how long the token lasts").into(),
                )
            })?)),
        };
        Ok(Self {
            secret,
            lifetime,
            expires: lifetime.and_then(|lifetime| now.checked_add(lifetime)),
        })
    }
}

/// Where a listener finds the token each new connection must present: one given once, or a
/// file read again for every connection, so the side that minted it can renew it (a worker
/// whose connection died is reached again with a fresh one). A token's lifetime counts from
/// when the listener first reads it.
#[derive(Debug, Clone)]
pub struct Tokens {
    file: Option<PathBuf>,
    /// The token in force: the one given, or the last one read from the file.
    current: Arc<Mutex<Option<Token>>>,
}

impl From<Token> for Tokens {
    fn from(token: Token) -> Self {
        Self {
            file: None,
            current: Arc::new(Mutex::new(Some(token))),
        }
    }
}

impl Tokens {
    /// The token in the file at `path`, as [`Token::to_file`] wrote it.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            file: Some(path.into()),
            current: Arc::new(Mutex::new(None)),
        }
    }

    /// The token a connection opened at `now` must present. A file holding the same secret
    /// as before keeps that token's expiry; a new secret starts its lifetime at `now`.
    pub async fn current(&self, now: SystemTime) -> std::io::Result<Token> {
        let held = || self.current.lock().ok().and_then(|current| current.clone());
        let Some(file) = &self.file else {
            return held().ok_or_else(|| std::io::Error::other("no token is set"));
        };
        let text = tokio::fs::read_to_string(file).await?;
        let read = Token::from_file(&text, now).map_err(std::io::Error::other)?;
        let mut current = self
            .current
            .lock()
            .map_err(|_| std::io::Error::other("the token's lock is poisoned"))?;
        match current.as_ref() {
            Some(token) if token.secret == read.secret => Ok(token.clone()),
            _ => Ok(current.insert(read).clone()),
        }
    }
}

/// Compares secrets in time that does not depend on where they differ.
pub(crate) fn same(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |differ, (a, b)| differ | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests;
