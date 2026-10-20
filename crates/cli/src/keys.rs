//! The root that wraps this computer's keys, so the database is sealed at rest.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use glave_keys::{KeyFileRoot, KeySettings, KeychainRoot, Root};

/// The keychain item that holds the root: the service and account names.
const KEYCHAIN: (&str, &str) = ("glave", "root key");

/// The key file's name in glave's folder.
const KEY_FILE: &str = "root.key";

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
