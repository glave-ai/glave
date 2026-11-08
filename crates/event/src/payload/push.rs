//! Push notifications' records (`specs/session.md`, "Push notifications"), in the push ledger:
//! in an organization's own scope, `push.key.made`, the key its server signs every push with,
//! the first one recorded holding for good; in a person's personal project, the browsers that
//! take their notifications, each `push.subscribed` until a `push.unsubscribed` of its endpoint
//! (the person turned it off, or its push service said it is gone). In their settings ledger,
//! `notifications.set`: which kinds they want, the latest replacing every one before it, and
//! with none, every kind.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Body, Payload, parse};
use crate::secret::Secret;

pub(crate) const KEY_MADE: &str = "push.key.made";
pub(crate) const SUBSCRIBED: &str = "push.subscribed";
pub(crate) const UNSUBSCRIBED: &str = "push.unsubscribed";
pub(crate) const SET: &str = "notifications.set";

/// The key an organization's server signs its pushes with (VAPID): its private half, which
/// only that server reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushKeyMade {
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub key: Secret,
}

/// A browser or device that takes the person's notifications, as its push subscription says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushSubscribed {
    /// Where its push service takes pushes for it.
    pub endpoint: String,
    /// The public key a push to it is encrypted to, in base64url.
    pub p256dh: String,
    /// The secret it shares with the server, in base64url.
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub auth: Secret,
    /// What the person knows it as: "Safari on iPhone".
    pub device: String,
}

/// A browser takes the person's notifications no more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushUnsubscribed {
    pub endpoint: String,
    /// Its push service said the subscription is gone, rather than the person turning it off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

/// Which kinds of notification the person wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationsSet {
    /// When something waits for them: an approval, a question, a service's ask.
    pub waiting: bool,
    /// When a task or a schedule's run finishes or fails.
    pub finished: bool,
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

impl Payload for NotificationsSet {
    const TYPE: &'static str = SET;
    const VERSION: u32 = 1;
}

/// Decodes the push ledger's events and `notifications.set`; `None` for any other type or
/// version.
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
        SET => parse(data).map(Body::NotificationsSet),
        _ => return None,
    })
}
