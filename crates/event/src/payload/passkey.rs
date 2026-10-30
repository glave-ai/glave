//! The passkeys of the person on their own server (`specs/session.md`, "Signing in"), kept in
//! the browsers ledger beside the sessions they sign in. `passkey.added`: a passkey was made
//! for a device, through a one-time setup link or by someone already signed in.
//! `passkey.used`: it signed a browser in, its credential as the authenticator now reports it
//! when that changed. `passkey.removed`: it signs no one in any more, and the sessions it began
//! end with it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::codec::{Payload, parse};

pub(crate) const ADDED: &str = "passkey.added";
pub(crate) const USED: &str = "passkey.used";
pub(crate) const REMOVED: &str = "passkey.removed";

/// A passkey was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasskeyAdded {
    /// The passkey, by the id that names it in sessions and when it is removed.
    pub passkey: Uuid,
    /// What it is called, for the person to tell their passkeys apart.
    pub name: String,
    /// The credential as the relying party keeps it: its id and public key, never a secret.
    pub credential: String,
    /// The relying party it was made for, by its id: the public origin's name, or `localhost`
    /// on the person's own computer. None for one made before it was kept, at a public origin,
    /// the only place passkeys were made then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party: Option<String>,
    /// Whether it was made from a link sent to the account's email, to get back in, rather
    /// than by someone signed in or with a link from the server's computer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub emailed: bool,
}

/// A passkey signed a browser in at the event's time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasskeyUsed {
    pub passkey: Uuid,
    /// Its credential as it now is, when signing in changed it (its counter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// A passkey was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasskeyRemoved {
    pub passkey: Uuid,
}

impl Payload for PasskeyAdded {
    const TYPE: &'static str = ADDED;
    const VERSION: u32 = 1;
}

impl Payload for PasskeyUsed {
    const TYPE: &'static str = USED;
    const VERSION: u32 = 1;
}

impl Payload for PasskeyRemoved {
    const TYPE: &'static str = REMOVED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_added(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PasskeyAdded>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_used(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PasskeyUsed>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_removed(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<PasskeyRemoved>> {
    (version == 1).then(|| parse(data))
}
