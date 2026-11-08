//! Push notifications as clients see them (`push/*`): whether the server sends them, the key a
//! browser subscribes with, and the browsers that take the person's.

use std::fmt;

use glave_event::{At, DeviceId, Secret};
use serde::{Deserialize, Serialize};

use super::Told;

/// A browser's push subscription, as `PushManager.subscribe` made it (`PushSubscription.toJSON`),
/// and what the person knows the browser as. Its `Debug` says neither its endpoint nor its
/// secret.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

impl fmt::Debug for PushSubscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PushSubscription")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

/// Whether the server tells the person's browsers when glave needs them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PushInfo {
    /// It sends none here, and `reason`, why.
    Never { reason: Told },
    /// It sends them: when something waits for the person (an approval, a question, a service's
    /// ask) and when a task of theirs finishes or fails.
    Offered {
        /// The key a browser subscribes with (`applicationServerKey`), in base64url.
        key: String,
        /// The browsers that take their notifications, the latest first.
        devices: Vec<PushDevice>,
    },
}

/// A browser that takes the person's notifications.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PushDevice {
    /// What `push/remove` names it by.
    pub id: DeviceId,
    /// What the person knows it as: "Safari on iPhone".
    pub name: String,
    pub added: At,
    /// It is the browser that asked: `push/get`'s `endpoint` is its subscription's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub current: bool,
}
