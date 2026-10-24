//! The root that wraps this computer's keys, so the database is sealed at rest, and what it
//! derives that must outlive the database: the person's passkey handle.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use glave_keys::{KeyFileRoot, KeySettings, KeychainRoot, Root};
use uuid::Uuid;

use crate::data::Data;

/// The keychain item that holds the root: the service and account names.
const KEYCHAIN: (&str, &str) = ("glave", "root key");

/// The key file's name in glave's folder.
const KEY_FILE: &str = "root.key";

/// What the root derives the local person's passkey handle for.
const PASSKEY_HANDLE: &str = "passkey user handle";

/// The passkey handle of the local person, whose root this is, which every passkey made for
/// them keeps (WebAuthn's user handle): derived from the root, so it is random, says nothing of
/// them, and lasts as long as the root does, through a fresh start of the store; a passkey made
/// again on a device then replaces the one made there before (`specs/session.md`, "Signing
/// in"). It is the root's, so it is for the one person the root is for, never another.
pub(crate) async fn local_passkey_handle(store: &Data) -> anyhow::Result<Uuid> {
    let derived = store.derived(PASSKEY_HANDLE).await?;
    let bytes = derived
        .bytes()
        .first_chunk::<16>()
        .copied()
        .context("the root derived too few bytes for a passkey handle")?;
    Ok(uuid::Builder::from_custom_bytes(bytes).into_uuid())
}

/// Keys for the database in `home`, glave's folder. The root lives in the OS keychain for the
/// default folder. A key file in the folder is used instead when the folder was moved with
/// `GLAVE_HOME` (each folder its own root), when one is already there, or, with a warning,
/// when this machine has no keychain.
pub(crate) async fn settings(home: &Path, relocated: bool) -> anyhow::Result<KeySettings> {
    let file = home.join(KEY_FILE);
    let root = tokio::task::spawn_blocking(move || -> anyhow::Result<Arc<dyn Root>> {
        if relocated || file.exists() {
            return key_file(&file);
        }
        if !KeychainRoot::available() {
            eprintln!(
                "glave: warning: no OS keychain here, so the root key is kept in {}, readable only by you",
                file.display()
            );
            return key_file(&file);
        }
        let root = KeychainRoot::open_or_create(KEYCHAIN.0, KEYCHAIN.1)
            .context("could not read glave's root key from the OS keychain")?;
        Ok(Arc::new(root))
    })
    .await??;
    Ok(KeySettings::new(root))
}

fn key_file(path: &Path) -> anyhow::Result<Arc<dyn Root>> {
    let root = KeyFileRoot::open_or_create(path)
        .with_context(|| format!("could not open the key file {}", path.display()))?;
    Ok(Arc::new(root))
}
