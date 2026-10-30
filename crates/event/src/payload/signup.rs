//! The sign-ups ledger's records (`specs/model.md`, "Identity"): on a server that takes
//! sign-ups, someone asked for a link to their email, kept as the SHA-256 of its code and of
//! the address, never either, with when it stops working (`signup.started`), and that link
//! spent when it made the person's account and organization (`signup.finished`). A link to get
//! back into an account (`recovery`) is kept the same way, and spent when it made a passkey. The link
//! carries the address; its code's hash, which covers it, proves it was sent there. One
//! stream of the server's own organization, so every server of a cell opens the same links,
//! and each once.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::envelope::At;

pub(crate) const STARTED: &str = "signup.started";
pub(crate) const FINISHED: &str = "signup.finished";

/// A sign-up's link sent to `email`, good until `expires`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignupStarted {
    /// The SHA-256 of its code, in hex: never the code.
    pub hash: String,
    /// The SHA-256 of the address its link was sent to, in hex: what counts the messages sent
    /// there, never the address.
    pub to: String,
    pub expires: At,
    /// Whether its link gets someone back into the account the address has, making a new
    /// passkey for it, rather than making an account.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recovery: bool,
}

/// A sign-up's link spent: its account and organization made, or its new passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignupFinished {
    pub hash: String,
}

impl Payload for SignupStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 1;
}

impl Payload for SignupFinished {
    const TYPE: &'static str = FINISHED;
    const VERSION: u32 = 1;
}

/// The sign-ups ledger's events, or `None` for a type or version that is none of them.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    if version != 1 {
        return None;
    }
    Some(match kind {
        STARTED => parse(data).map(Body::SignupStarted),
        FINISHED => parse(data).map(Body::SignupFinished),
        _ => return None,
    })
}
