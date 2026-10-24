//! Signing in from elsewhere with passkeys (`specs/session.md`, "Signing in"): how a server
//! says it signs browsers in, a passkey as a client lists it, and what a browser tells its
//! passkey manager of the person's passkeys (WebAuthn's Signal API).

use glave_event::At;
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

/// Every passkey that signs the person in with a relying party, after a sign-in, for the
/// browser to tell its passkey manager (`PublicKeyCredential.signalAllAcceptedCredentials`),
/// which then hides or deletes the person's others there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AcceptedPasskeys {
    /// The relying party's id (`rpId`): the public origin's name, or `localhost`.
    pub party: String,
    /// The person's user handle (`userId`), in base64url.
    pub user: String,
    /// Each passkey's credential id (`allAcceptedCredentialIds`), in base64url.
    pub credentials: Vec<String>,
}

/// A passkey a browser signed in with that the server does not know, removed or never made
/// for it, for the browser to tell its passkey manager
/// (`PublicKeyCredential.signalUnknownCredential`), which then deletes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UnknownPasskey {
    /// The relying party's id (`rpId`).
    pub party: String,
    /// Its credential id (`credentialId`), in base64url.
    pub credential: String,
}
