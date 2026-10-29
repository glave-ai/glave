//! Signing in from elsewhere with passkeys (`specs/session.md`, "Signing in"): how a server
//! says it signs browsers in, a passkey as a client lists it, and what a browser tells its
//! passkey manager of the person's passkeys (WebAuthn's Signal API).

use glave_event::At;
pub use glave_identity::signin::{AcceptedPasskeys, UnknownPasskey};
use serde::{Deserialize, Serialize};

/// A way a browser signs in to the server, as `server/discover` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignInWay {
    /// A link glave on the server's own computer makes, good once, for a browser there.
    Link,
    /// A passkey: at the server's public origin, where a setup link from glave on its computer
    /// makes the first; or on this computer, once a browser signed in by glave's link made one.
    Passkey,
    /// Signing up with an email: a link sent there makes the person's first passkey, their
    /// account and an organization of their own.
    Signup,
}

/// Which link's code a passkey is made with (`passkey/create_options`).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// A setup link from glave on the server's computer: the local person's passkey.
    #[default]
    Setup,
    /// An invite's: the first passkey of a new account, joining its organization.
    Invite,
    /// A sign-up's, emailed: the first passkey of a new account and its own organization.
    Signup,
}

impl LinkKind {
    /// Whether it is a setup link's, the one a request names by saying none.
    pub(crate) fn is_setup(&self) -> bool {
        *self == Self::Setup
    }
}

/// A passkey that signs the person in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PasskeySummary {
    pub id: String,
    /// What it is called: the name the browser gave it, else when it was added.
    pub name: String,
    pub added: At,
    /// When it last signed a browser in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<At>,
}
