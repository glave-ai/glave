//! glave's Web Push (`specs/session.md`, "Push notifications"): one short message to one of a
//! person's browsers, through the push service that browser chose (Google's, Mozilla's,
//! Apple's, Microsoft's), so it arrives with the app closed. The standard alone: the server
//! signs each push with its own key (VAPID, RFC 8292), and encrypts what it says to the
//! browser's keys (RFC 8291, through web-push-native), so the push service relays it but cannot
//! read it. glave runs no notification server of its own.

#[cfg(test)]
mod example;
mod key;
mod send;
mod subscription;

pub use key::{KeyError, VapidKey};
pub use send::{Message, PushError, Pusher, Sent, Urgency};
pub use subscription::{Refused, Subscription};
