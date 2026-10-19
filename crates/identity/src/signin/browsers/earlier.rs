//! The file an earlier glave kept browser sessions in, `browsers.json` in its folder: read once
//! so its server's id and sessions move into the store, then deleted.

use std::path::Path;

use jiff::Timestamp;
use serde::Deserialize;
use uuid::Uuid;

use super::{BrowserError, SESSION_LIFETIME};

/// The only version of the file there was.
const VERSION: u32 = 1;

/// What the file held.
#[derive(Deserialize)]
pub struct Earlier {
    version: u32,
    /// The server's id, the same across restarts.
    pub server: String,
    sessions: Vec<EarlierSession>,
}

#[derive(Deserialize)]
struct EarlierSession {
    id: String,
    /// The SHA-256 of the token, in hex.
    hash: String,
    /// When it was last used, to the hour.
    used: Timestamp,
}

impl Earlier {
    /// The file at `path`; none when there is none, or none glave can read, which ends every
    /// session it held as an unreadable file always did.
    pub fn read(path: &Path) -> Result<Option<Self>, BrowserError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str::<Self>(&text)
                .ok()
                .filter(|earlier| earlier.version == VERSION)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(BrowserError::Keep(format!("{}: {error}", path.display()))),
        }
    }

    /// The sessions still good at `now`: each one's id and its token's hash.
    pub(super) fn live(&self, now: Timestamp) -> impl Iterator<Item = (Uuid, &str)> {
        self.sessions
            .iter()
            .filter(move |session| now.duration_since(session.used) <= SESSION_LIFETIME)
            .filter_map(|session| {
                let id = Uuid::parse_str(&session.id).ok()?;
                Some((id, session.hash.as_str()))
            })
    }
}
