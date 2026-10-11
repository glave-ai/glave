//! The token a client of `glave computer serve` presents first: a long-lived one a person
//! sets, or a short-lived one the harness side mints for a worker, which only opens
//! connections until it expires.

use std::fmt::{self, Write as _};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use glave_ports::ComputerError;

/// Random bytes in a minted token.
const MINTED_BYTES: usize = 32;
/// The shortest secret a listener accepts.
pub const MIN_SECRET: usize = 16;

/// A secret, and when it stops opening connections. A connection opened before then lasts.
#[derive(Clone, PartialEq, Eq)]
pub struct Token {
    secret: String,
    /// `None` for a token that does not expire.
    expires: Option<SystemTime>,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("secret", &"<hidden>")
            .field("expires", &self.expires)
            .finish()
    }
}

impl Token {
    /// A token that never expires, such as one a person sets in `GLAVE_COMPUTER_TOKEN`.
    pub fn lasting(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
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

    /// The token as a file holds it: the secret, then the second it expires (Unix time), each
    /// on its own line.
    pub fn to_file(&self) -> String {
        let expires = self
            .expires
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or_else(String::new, |since| since.as_secs().to_string());
        format!("{}\n{expires}\n", self.secret)
    }

    /// A token read from a file [`Token::to_file`] wrote; refused when the secret is too short
    /// to guard a listener.
    pub fn from_file(text: &str) -> Result<Self, ComputerError> {
        let mut lines = text.lines();
        let secret = lines.next().unwrap_or_default().trim().to_owned();
        if secret.len() < MIN_SECRET {
            return Err(ComputerError::Refused(format!(
                "the token is shorter than {MIN_SECRET} characters"
            )));
        }
        let expires = match lines.next().map(str::trim).filter(|line| !line.is_empty()) {
            None => None,
            Some(seconds) => {
                let seconds: u64 = seconds.parse().map_err(|_| {
                    ComputerError::Refused(format!("{seconds:?} is not a time the token expires"))
                })?;
                UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
            }
        };
        Ok(Self { secret, expires })
    }
}

/// Compares secrets in time that does not depend on where they differ.
fn same(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |differ, (a, b)| differ | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn a_minted_token_opens_connections_only_until_it_expires() {
        let token = Token::mint(at(1_000), Duration::from_secs(60)).unwrap();
        assert_eq!(token.secret().len(), 64);
        assert!(token.admits(token.secret(), at(1_059)));
        assert!(!token.admits(token.secret(), at(1_060)), "expired");
        assert!(!token.admits("guess", at(1_001)));
        let other = Token::mint(at(1_000), Duration::from_secs(60)).unwrap();
        assert_ne!(token.secret(), other.secret(), "random every time");
        assert!(!format!("{token:?}").contains(token.secret()));
    }

    #[test]
    fn tokens_survive_their_file_and_short_ones_are_refused() {
        let token = Token::mint(at(5), Duration::from_secs(10)).unwrap();
        let back = Token::from_file(&token.to_file()).unwrap();
        assert_eq!(back, token);
        assert_eq!(back.expires(), Some(at(15)));
        let lasting = Token::from_file("0123456789abcdef\n").unwrap();
        assert!(lasting.admits("0123456789abcdef", at(u64::from(u32::MAX))));
        assert!(Token::from_file("short\n").is_err());
        assert!(Token::from_file("0123456789abcdef\nsoon\n").is_err());
    }

    #[test]
    fn secrets_compare_whole() {
        assert!(same("abc", "abc"));
        assert!(!same("abd", "abc"));
        assert!(!same("ab", "abc"));
    }
}
