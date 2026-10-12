//! glave's keys: content at rest is sealed, and erased by destroying the keys that sealed it.
//!
//! The hierarchy (`specs/storage.md`, "Keys"): a root wraps each organization's key, the
//! organization key wraps its spaces' keys, and a space key wraps one data key per month,
//! which seals the content. The data store keeps only wrapped keys, through [`KeyStorage`];
//! [`Keys`] unwraps them on demand and caches the results in memory, up to a fixed count.
//!
//! Sealing is XChaCha20-Poly1305 with a random 24-byte nonce, bound by its associated data to
//! the row it was written for, so a sealed payload cannot be moved between rows, threads or
//! scopes.

#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
mod error;
mod key_id;
#[cfg(feature = "keychain")]
mod keychain;
mod keys;
mod record;
mod root;
mod seal;
mod secret;
mod storage;

pub use error::KeyError;
pub use key_id::{KeyId, Month, Period};
#[cfg(feature = "keychain")]
pub use keychain::KeychainRoot;
pub use keys::{EventFields, KeySettings, Keys, Sealed, erased_payload};
pub use root::{KeyFileRoot, MemoryRoot, Root};
pub use secret::Secret;
pub use storage::{KeyStorage, MemoryKeyStorage, StoredSpace};
