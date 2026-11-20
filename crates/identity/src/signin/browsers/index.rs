//! The server-wide index of glave accounts' passkeys (#1523): each passkey's credential id, as
//! a hash in the directory, leads to the account it was made for. Signing in with a passkey
//! without saying whose (`passkeys::discoverable`) looks the credential up here first, so an
//! answer whose credential no account has is refused before any person's ledger is opened,
//! whatever person id it names, and how long the refusal takes says nothing of whether that id
//! has an account. An account's passkeys are indexed as it is opened and as each is added.

use std::sync::Arc;

use glave_ports::{Directory, IdentityKey};

use super::{BrowserError, Browsers, digest, hex};

/// The directory key a passkey whose credential id is `credential` is indexed under: the
/// SHA-256 of the id, apart from every other kind of identity.
pub(crate) fn passkey_key(credential: &[u8]) -> Result<IdentityKey, BrowserError> {
    IdentityKey::new(&hex(&digest(&format!("passkey\0{}", hex(credential)))))
        .map_err(|e| BrowserError::Keep(e.to_string()))
}

impl Browsers {
    /// Indexes every passkey of this glave account in `index`, so each leads to the account:
    /// those it had before the index, and those added since. One indexed already is left as it
    /// is.
    pub(crate) async fn index_passkeys(
        &self,
        index: &Arc<dyn Directory>,
    ) -> Result<(), BrowserError> {
        let Some(person) = self.account else {
            return Ok(());
        };
        let credentials: Vec<Vec<u8>> = self
            .state()
            .passkeys
            .iter()
            .map(|kept| kept.credential.cred_id().as_slice().to_vec())
            .collect();
        let failed = |e: glave_ports::DirectoryError| BrowserError::Keep(e.to_string());
        for credential in credentials {
            let key = passkey_key(&credential)?;
            // One indexed already is only read, so a start after the first writes nothing.
            if index.find(&key).await.map_err(failed)? != Some(person) {
                index.link(&key, person).await.map_err(failed)?;
            }
        }
        Ok(())
    }
}
