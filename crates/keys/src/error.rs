//! Why a key could not be made, found, unwrapped or used.

use thiserror::Error;

/// Why sealing, opening or a key operation failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyError {
    /// The root could not be reached or refused to wrap or unwrap.
    #[error("the root key is unavailable: {0}")]
    Root(String),
    /// The key was destroyed, so what it sealed is erased.
    #[error("key {0} was destroyed: the content it sealed is erased")]
    Destroyed(String),
    /// The key never existed in this project.
    #[error("key {0} is not known in this project")]
    Unknown(String),
    /// Sealed bytes did not open: altered, moved to another row, or sealed under another key.
    #[error("sealed content under key {0} did not open: it was altered or does not belong here")]
    Tampered(String),
    /// A `key_id` that does not follow `v1/<project>/<YYYY-MM>/<n>`.
    #[error("{0:?} is not a valid key id")]
    InvalidKeyId(String),
    /// Stored key material that cannot be read.
    #[error("stored keys are corrupt: {0}")]
    Corrupt(String),
    /// Another writer kept changing the project's keys.
    #[error("the keys of {0} kept changing while being updated; try again")]
    Contended(String),
    /// The operating system could not supply random bytes.
    #[error("no randomness available: {0}")]
    Random(String),
    /// The storage behind the keys failed.
    #[error("key storage failed: {0}")]
    Storage(String),
}
