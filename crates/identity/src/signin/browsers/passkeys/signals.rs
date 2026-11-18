//! What a browser tells its passkey manager of the person's passkeys (WebAuthn's Signal API), as
//! the server says it: after a sign-in, every passkey that still signs them in
//! (`signalAllAcceptedCredentials`), and of a passkey presented that this server does not know
//! (`signalUnknownCredential`), so the manager hides or deletes the dead ones itself. Ids go as
//! base64url, as the browser takes them. These are the session API's own types too.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use webauthn_rs::prelude::PublicKeyCredential;

use super::{HERE, Party, Relying};
use crate::signin::browsers::{BrowserError, Browsers};

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

/// A passkey a browser signed in with that the server does not know, removed since the
/// sign-in began, for the browser to tell its passkey manager
/// (`PublicKeyCredential.signalUnknownCredential`), which then deletes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UnknownPasskey {
    /// The relying party's id (`rpId`).
    pub party: String,
    /// Its credential id (`credentialId`), in base64url.
    pub credential: String,
}

impl Browsers {
    /// The passkeys that sign the person in, after a sign-in with a passkey (`by_passkey`) or
    /// a link: none where the server makes no passkeys, nor after a link where its relying
    /// party is its public origin, since a link's page is on this computer, not at that origin.
    pub(in crate::signin::browsers) fn accepted(
        &self,
        by_passkey: bool,
    ) -> Option<AcceptedPasskeys> {
        let relying = self.relying.get()?;
        if !by_passkey && matches!(relying.party, Party::Public) {
            return None;
        }
        let credentials = self
            .state()
            .passkeys
            .iter()
            .filter(|kept| relying.made(kept))
            .map(|kept| URL_SAFE_NO_PAD.encode(kept.credential.cred_id()))
            .collect();
        Some(AcceptedPasskeys {
            party: relying.id.clone(),
            user: URL_SAFE_NO_PAD.encode(relying.handle.as_bytes()),
            credentials,
        })
    }
}

impl Relying {
    /// Why `answer`, with a passkey this server does not know, is refused: as
    /// [`BrowserError::Unknown`] when it carries the person's handle, so their passkey manager
    /// can forget it; plainly when it carries another's or none, as a passkey of another
    /// program's at `localhost` may, which is not this server's to judge.
    pub(super) fn unknown(&self, answer: &PublicKeyCredential) -> BrowserError {
        if answer.get_user_unique_id() == Some(self.handle.as_bytes().as_slice()) {
            forget(&self.id, answer)
        } else {
            not_glaves()
        }
    }

    /// Why `answer`, with a passkey no glave account here knows, is refused, whoever's handle
    /// it carries, so the answer never says whether that handle is an account's: as
    /// [`BrowserError::Unknown`] where the relying party is a name the server alone makes
    /// passkeys for; plainly at `localhost`, every local program's, where another's passkey is
    /// not this server's to judge.
    pub(super) fn no_account_knows(&self, answer: &PublicKeyCredential) -> BrowserError {
        if self.id == HERE {
            not_glaves()
        } else {
            forget(&self.id, answer)
        }
    }
}

/// `answer`'s passkey, of the relying party `party`, refused as one the server does not know,
/// for the browser's passkey manager to forget.
fn forget(party: &str, answer: &PublicKeyCredential) -> BrowserError {
    BrowserError::Unknown(UnknownPasskey {
        party: party.to_owned(),
        credential: URL_SAFE_NO_PAD.encode(answer.get_credential_id()),
    })
}

/// A passkey refused as not this server's, which no passkey manager is told to forget.
fn not_glaves() -> BrowserError {
    BrowserError::Passkey("it is not one of this glave's".into())
}
