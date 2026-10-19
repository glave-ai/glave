//! The browsers ledger's records (`specs/one-server.md`, "Finding it"): the browser sessions
//! of the person on their own server. `browser.signed_in`: a link's code was traded for a
//! session, kept as its token's SHA-256 alone. `browser.used`: a session was used, which keeps
//! it going for 30 days from then. `browser.signed_out`: sessions ended.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};

pub(crate) const SIGNED_IN: &str = "browser.signed_in";
pub(crate) const USED: &str = "browser.used";
pub(crate) const SIGNED_OUT: &str = "browser.signed_out";

/// A browser signed in: a session began, used from the event's time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSignedIn {
    /// The session, by the id that names it in records and when it is ended.
    pub session: Uuid,
    /// The SHA-256 of its token, in hex: never the token.
    pub hash: String,
}

/// A browser session was used at the event's time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserUsed {
    pub session: Uuid,
}

/// Browser sessions ended: signed out, one or every other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSignedOut {
    pub sessions: Vec<Uuid>,
}

impl Payload for BrowserSignedIn {
    const TYPE: &'static str = SIGNED_IN;
    const VERSION: u32 = 1;
}

impl Payload for BrowserUsed {
    const TYPE: &'static str = USED;
    const VERSION: u32 = 1;
}

impl Payload for BrowserSignedOut {
    const TYPE: &'static str = SIGNED_OUT;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_signed_in(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<BrowserSignedIn>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_used(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<BrowserUsed>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_signed_out(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<BrowserSignedOut>> {
    (version == 1).then(|| parse(data))
}
