//! The OS keychain as a person's root: macOS Keychain, Windows Credential Manager, or the
//! Secret Service on Linux, through the `keyring` crate.

use keyring::{Entry, Error};

use crate::error::KeyError;
use crate::root::{LocalRoot, local_root};
use crate::secret::Secret;

/// A root whose key is one keychain item, created on first use.
#[derive(Debug, Clone)]
pub struct KeychainRoot {
    local: LocalRoot,
}

impl KeychainRoot {
    /// Whether this machine has a keychain to keep the root in. A headless Linux machine
    /// without a Secret Service has none.
    pub fn available() -> bool {
        Entry::store_status().is_ok()
    }

    /// Reads the root key from the keychain item named `service` and `account`, creating it
    /// with a new random key if it does not exist. Fails when there is no keychain to reach,
    /// as on a headless Linux machine. This talks to the OS and may wait on the person (an
    /// unlock prompt); call it off the async runtime.
    pub fn open_or_create(service: &str, account: &str) -> Result<Self, KeyError> {
        let failed = |e: Error| KeyError::Root(format!("the keychain: {e}"));
        let entry = Entry::new(service, account).map_err(failed)?;
        let key = match entry.get_secret() {
            Ok(bytes) => Secret::from_bytes(&zeroize::Zeroizing::new(bytes))?,
            Err(Error::NoEntry) => {
                entry
                    .set_secret(Secret::random()?.bytes())
                    .map_err(failed)?;
                // Read back what was kept, so two first starts at once agree on one key.
                let kept = zeroize::Zeroizing::new(entry.get_secret().map_err(failed)?);
                Secret::from_bytes(&kept)?
            }
            Err(e) => return Err(failed(e)),
        };
        Ok(Self {
            local: LocalRoot::new(key),
        })
    }
}

local_root!(KeychainRoot);
