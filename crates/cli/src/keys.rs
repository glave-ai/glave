//! The root that wraps this computer's keys, so the database is sealed at rest, and what it
//! derives that must outlive the database: the person's passkey handle, and the secret each
//! person's anonymous id for model providers is made under.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use glave_keys::{KeyFileRoot, KeySettings, KeychainRoot, Root};
use glave_ports::EndUsers;
use uuid::Uuid;

use crate::data::Data;
use crate::said::Said;

/// The keychain item that holds the root: the service and account names.
const KEYCHAIN: (&str, &str) = ("glave", "root key");

/// The key file's name in glave's folder.
const KEY_FILE: &str = "root.key";

/// What the root derives the local person's passkey handle for.
const PASSKEY_HANDLE: &str = "passkey user handle";

/// What the root derives the secret of people's anonymous ids for model providers for.
const END_USERS: &str = "model provider end user ids";

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

/// What makes each person's anonymous id for model providers (`specs/gateway.md`, "Whose call
/// it is"): under a secret the root derives, so only this server can make or recognise one, and
/// an id stays the same through a fresh start of the store.
pub(crate) async fn end_users(store: &Data) -> anyhow::Result<EndUsers> {
    let derived = store.derived(END_USERS).await?;
    Ok(EndUsers::new(derived.bytes()))
}

/// Keys for the database in `home`, glave's folder. The root lives in the OS keychain for the
/// default folder. A key file in the folder is used instead when the folder was moved with
/// `GLAVE_HOME` (each folder its own root), when one is already there, or, with a warning said
/// as `said`, when this machine has no keychain.
pub(crate) async fn settings(
    home: &Path,
    relocated: bool,
    said: Said,
) -> anyhow::Result<KeySettings> {
    let file = home.join(KEY_FILE);
    let (root, no_keychain) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        if relocated || file.exists() {
            return Ok((key_file(&file)?, None));
        }
        if !KeychainRoot::available() {
            return Ok((key_file(&file)?, Some(file)));
        }
        let root: Arc<dyn Root> = Arc::new(
            KeychainRoot::open_or_create(KEYCHAIN.0, KEYCHAIN.1)
                .context("could not read glave's root key from the OS keychain")?,
        );
        Ok((root, None))
    })
    .await??;
    // Said here, not on the thread that read it, which the log is not set up on.
    if let Some(file) = no_keychain {
        let file = file.display();
        match said {
            Said::Plainly => eprintln!(
                "glave: warning: no OS keychain here, so the root key is kept in {file}, \
                 readable only by you"
            ),
            Said::Logged => tracing::warn!(
                %file,
                "no OS keychain here, so the root key is kept in a file readable only by glave"
            ),
        }
    }
    Ok(KeySettings::new(root))
}

fn key_file(path: &Path) -> anyhow::Result<Arc<dyn Root>> {
    let root = KeyFileRoot::open_or_create(path)
        .with_context(|| format!("could not open the key file {}", path.display()))?;
    Ok(Arc::new(root))
}
