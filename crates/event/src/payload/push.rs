//! Push notifications' records (`specs/session.md`, "Push notifications"), under the push
//! ledger's id: in the server's own organization's scope, `push.key.made`, the key every push
//! on the store is signed with, the first one recorded holding for good, and, in a stream of
//! each endpoint's own, `push.claimed`, the person whose browser it is now; in a person's
//! personal project, the browsers that take their notifications, each `push.subscribed`, sealed
//! under a key of its own, until a `push.unsubscribed` of it erases it (the person turned it off,
//! it moved to someone else, or its push service said it is gone or kept failing).

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::id::{CredentialId, PrincipalId};
use crate::secret::Secret;

pub(crate) const KEY_MADE: &str = "push.key.made";
pub(crate) const SUBSCRIBED: &str = "push.subscribed";
pub(crate) const UNSUBSCRIBED: &str = "push.unsubscribed";
pub(crate) const CLAIMED: &str = "push.claimed";

/// The key the server signs every push with (VAPID): its private half, which only the store's
/// servers read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushKeyMade {
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub key: Secret,
}

/// A browser or device that takes the person's notifications, as its push subscription says.
/// Sealed under the device's own key, so forgetting it erases it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushSubscribed {
    /// What it is kept and erased by.
    pub device: CredentialId,
    /// Where its push service takes pushes for it.
    pub endpoint: String,
    /// The public key a push to it is encrypted to, in base64url.
    pub p256dh: String,
    /// The secret it shares with the server, in base64url.
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub auth: Secret,
    /// What the person knows it as: "Safari on iPhone".
    pub name: String,
}

/// A browser takes the person's notifications no more, and is erased.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushUnsubscribed {
    pub device: CredentialId,
    /// Its push service said the subscription is gone, or kept failing, rather than the person
    /// turning it off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

/// Whose browser an endpoint is now: the person who subscribed it last, as `device`. Holds no
/// endpoint: its stream is named by the endpoint's hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushClaimed {
    pub person: PrincipalId,
    pub device: CredentialId,
}

impl Payload for PushKeyMade {
    const TYPE: &'static str = KEY_MADE;
    const VERSION: u32 = 1;
}

impl Payload for PushSubscribed {
    const TYPE: &'static str = SUBSCRIBED;
    const VERSION: u32 = 1;
}

impl Payload for PushUnsubscribed {
    const TYPE: &'static str = UNSUBSCRIBED;
    const VERSION: u32 = 1;
}

impl Payload for PushClaimed {
    const TYPE: &'static str = CLAIMED;
    const VERSION: u32 = 1;
}

/// The device whose own key seals `data`, a `push.subscribed`'s.
pub(crate) fn sealed_device(data: &RawValue) -> Option<CredentialId> {
    #[derive(Deserialize)]
    struct Device {
        device: CredentialId,
    }
    serde_json::from_str::<Device>(data.get())
        .ok()
        .map(|kept| kept.device)
}

/// Decodes the push ledger's events; `None` for any other type or version.
pub(crate) fn decode(
    kind: &str,
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<Body>> {
    if version != 1 {
        return None;
    }
    Some(match kind {
        KEY_MADE => parse(data).map(Body::PushKeyMade),
        SUBSCRIBED => parse(data).map(Body::PushSubscribed),
        UNSUBSCRIBED => parse(data).map(Body::PushUnsubscribed),
        CLAIMED => parse(data).map(Body::PushClaimed),
        _ => return None,
    })
}
