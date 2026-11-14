//! The sign-ups ledger's records (`specs/model.md`, "Identity"): on a server that takes
//! sign-ups, someone asked for a link to their email, kept as the SHA-256 of its code and of
//! the address, never either, with when it stops working (`signup.started`), and that link
//! spent when it made the person's account and organization (`signup.finished`). A link to get
//! back into an account (`recovery`) is kept the same way, and spent when it made a passkey; so
//! is a sign-up link whoever runs the server made to hand to someone, which carries no address
//! (no `to`) and opens while the server takes no sign-ups. An emailed link carries the
//! address; its code's hash, which covers it, proves it was sent there. A link whoever runs
//! the server made may carry the balance the personal space it makes starts with (`credits`,
//! v3), and a sign-up spent names the organization it made (`organization`, v2), so whoever
//! runs the server can find the personal spaces sign-ups made. Every email
//! someone asks glave for (a link, or an invite) counts against their network, kept as the
//! SHA-256 of it (`mail.asked`). One stream of the server's own organization, so every server
//! of a cell opens the same links, each once, and counts each network alike.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Body, Payload, parse};
use crate::envelope::At;
use crate::money::Money;

pub(crate) const STARTED: &str = "signup.started";
pub(crate) const FINISHED: &str = "signup.finished";
pub(crate) const ASKED: &str = "mail.asked";

/// A sign-up's link sent to `email`, good until `expires`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignupStarted {
    /// The SHA-256 of its code, in hex: never the code.
    pub hash: String,
    /// The SHA-256 of the address its link was sent to, in hex: what counts the messages sent
    /// there, never the address. None for a sign-up link whoever runs the server made, from its
    /// computer, to hand to someone: it was sent nowhere, and opens while the server takes no
    /// sign-ups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub expires: At,
    /// Whether its link gets someone back into the account the address has, making a new
    /// passkey for it, rather than making an account.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recovery: bool,
    /// What the personal space a link whoever runs the server made starts with, added to its
    /// credits as it is made (v3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<Money>,
}

/// `signup.started` at version 2, before a link carried a starting balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignupStartedV2 {
    hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    expires: At,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    recovery: bool,
}

impl From<SignupStartedV2> for SignupStarted {
    fn from(old: SignupStartedV2) -> Self {
        Self {
            hash: old.hash,
            to: old.to,
            expires: old.expires,
            recovery: old.recovery,
            credits: None,
        }
    }
}

/// `signup.started` at version 1, when every link was emailed to an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignupStartedV1 {
    hash: String,
    to: String,
    expires: At,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    recovery: bool,
}

impl From<SignupStartedV1> for SignupStarted {
    fn from(old: SignupStartedV1) -> Self {
        Self {
            hash: old.hash,
            to: Some(old.to),
            expires: old.expires,
            recovery: old.recovery,
            credits: None,
        }
    }
}

/// A sign-up's link spent: its account and organization made, or its new passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignupFinished {
    pub hash: String,
    /// The organization a sign-up made, the person's personal space: none for a link to get
    /// back in, and for one spent before v2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<Uuid>,
}

/// `signup.finished` at version 1, before it named the organization made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignupFinishedV1 {
    hash: String,
}

impl From<SignupFinishedV1> for SignupFinished {
    fn from(old: SignupFinishedV1) -> Self {
        Self {
            hash: old.hash,
            organization: None,
        }
    }
}

/// An email asked for from a network: a sign-up's link, a link to get back in, or an invite,
/// which counts against what that network may ask for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailAsked {
    /// The SHA-256 of the network it was asked from, in hex: an IPv4 address, or the /64 an
    /// IPv6 address is in. Not the address as written, but as telling as it, since an IPv4
    /// address's few values can all be tried: kept sealed with the rest of the ledger.
    pub from: String,
    /// For an IPv6 address, the SHA-256 of the /48 its /64 is in, which counts too: one
    /// person can be given a whole /48, and every /64 in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
}

impl Payload for SignupStarted {
    const TYPE: &'static str = STARTED;
    const VERSION: u32 = 3;
}

impl Payload for SignupFinished {
    const TYPE: &'static str = FINISHED;
    const VERSION: u32 = 2;
}

impl Payload for MailAsked {
    const TYPE: &'static str = ASKED;
    const VERSION: u32 = 1;
}

/// The sign-ups ledger's events, or `None` for a type or version that is none of them.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    Some(match (kind, version) {
        (STARTED, 1) => parse::<SignupStartedV1>(data).map(|old| Body::SignupStarted(old.into())),
        (STARTED, 2) => parse::<SignupStartedV2>(data).map(|old| Body::SignupStarted(old.into())),
        (STARTED, 3) => parse(data).map(Body::SignupStarted),
        (FINISHED, 1) => {
            parse::<SignupFinishedV1>(data).map(|old| Body::SignupFinished(old.into()))
        }
        (FINISHED, 2) => parse(data).map(Body::SignupFinished),
        (ASKED, 1) => parse(data).map(Body::MailAsked),
        _ => return None,
    })
}
