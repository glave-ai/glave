//! Push notifications as clients see them (`push/*`): whether the server sends them, the key a
//! browser subscribes with, which kinds the person wants, and the browsers that take theirs.

use glave_event::{At, EventId, Secret};
use serde::{Deserialize, Serialize};

use super::Told;

/// A browser's push subscription, as `PushManager.subscribe` made it (`PushSubscription.toJSON`),
/// and what the person knows the browser as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PushSubscription {
    /// Where its push service takes pushes for it.
    pub endpoint: String,
    /// Its public key, in base64url.
    pub p256dh: String,
    /// The secret it shares with the server, in base64url.
    #[serde(serialize_with = "glave_event::serialize_exposed")]
    pub auth: Secret,
    /// What the person knows it as: "Safari on iPhone".
    pub device: String,
}

/// Whether the server tells the person's browsers when glave needs them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PushInfo {
    /// It sends none here, and `reason`, why.
    Never { reason: Told },
    /// It sends them.
    Offered {
        /// The key a browser subscribes with (`applicationServerKey`), in base64url.
        key: String,
        /// Whether they are told when something waits for them: an approval, a question, a
        /// service's ask.
        waiting: bool,
        /// Whether they are told when a task or a schedule's run finishes or fails.
        finished: bool,
        /// The browsers that take their notifications, the latest first.
        devices: Vec<PushDevice>,
    },
}

/// A browser that takes the person's notifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PushDevice {
    /// What `push/remove` names it by.
    pub id: EventId,
    /// What the person knows it as: "Safari on iPhone".
    pub name: String,
    pub added: At,
    /// It is the browser that asked: `push/get`'s `endpoint` is its subscription's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub current: bool,
}
