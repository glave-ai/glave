//! The sign-ups ledger's records (`specs/model.md`, "Identity"): on a server that takes
//! sign-ups, someone asked for a link to their email, kept as the SHA-256 of its code with the
//! address and when it stops working (`signup.started`), and that link spent when it made the
//! person's account and organization (`signup.finished`). One stream of the server's own
//! organization, so every server of a cell opens the same links, and each once.

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
    /// Where its link was sent: the email of the account it makes.
    pub email: String,
    pub expires: At,
}

/// A sign-up's link spent: its account and organization made.
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
