//! Roots: what wraps each organization's key. A person's root lives in the OS keychain (see
//! `KeychainRoot`), a headless machine's in a key file; an organization's will be a KMS.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Path, PathBuf};

use glave_ports::{BoxFuture, OrgId};

use crate::error::KeyError;
use crate::material::KeyMaterial;
use crate::seal::{associated, open, seal};

/// Wraps and unwraps organization keys. It is called only when an organization key is not
/// already in memory, so it may be slow or remote (a KMS).
pub trait Root: Send + Sync {
    /// Wraps `key`, the key of `org`, for the data store to keep.
    fn wrap<'a>(
        &'a self,
        org: OrgId,
        key: &'a KeyMaterial,
    ) -> BoxFuture<'a, Result<Vec<u8>, KeyError>>;

    /// Unwraps what [`Root::wrap`] returned for `org`.
    fn unwrap<'a>(
        &'a self,
        org: OrgId,
        wrapped: &'a [u8],
    ) -> BoxFuture<'a, Result<KeyMaterial, KeyError>>;
}

/// A root held as one 256-bit key in this process: wrapping is sealing under it, bound to the
/// organization. The keychain and key-file roots differ only in where that key is kept.
#[derive(Debug, Clone)]
pub(crate) struct LocalRoot {
    key: KeyMaterial,
}

impl LocalRoot {
    pub(crate) fn new(key: KeyMaterial) -> Self {
        Self { key }
    }

    fn aad(org: OrgId) -> Vec<u8> {
        associated(&[b"glave/org-key/v1", org.to_string().as_bytes()])
    }

    pub(crate) fn wrap(&self, org: OrgId, key: &KeyMaterial) -> Result<Vec<u8>, KeyError> {
        seal(&self.key, &Self::aad(org), key.bytes())
    }

    pub(crate) fn unwrap(&self, org: OrgId, wrapped: &[u8]) -> Result<KeyMaterial, KeyError> {
        let bytes = zeroize::Zeroizing::new(open(
            &self.key,
            &Self::aad(org),
            wrapped,
            &format!("org/{org}"),
        )?);
        KeyMaterial::from_bytes(&bytes)
    }
}

/// Implements [`Root`] for a type that holds a [`LocalRoot`] in a field named `local`.
macro_rules! local_root {
    ($type:ty) => {
        impl $crate::root::Root for $type {
            fn wrap<'a>(
                &'a self,
                org: glave_ports::OrgId,
                key: &'a $crate::material::KeyMaterial,
            ) -> glave_ports::BoxFuture<'a, Result<Vec<u8>, $crate::error::KeyError>> {
                Box::pin(std::future::ready(self.local.wrap(org, key)))
            }

            fn unwrap<'a>(
                &'a self,
                org: glave_ports::OrgId,
                wrapped: &'a [u8],
            ) -> glave_ports::BoxFuture<
                'a,
                Result<$crate::material::KeyMaterial, $crate::error::KeyError>,
            > {
                Box::pin(std::future::ready(self.local.unwrap(org, wrapped)))
            }
        }
    };
}
#[cfg(feature = "keychain")]
pub(crate) use local_root;

/// A root that lives only in this process: everything it wraps is unreadable once the
/// process ends. For tests and in-memory stores.
#[derive(Debug, Clone)]
pub struct InMemoryRoot {
    local: LocalRoot,
}

impl InMemoryRoot {
    pub fn new() -> Result<Self, KeyError> {
        Ok(Self {
            local: LocalRoot::new(KeyMaterial::random()?),
        })
    }
}

local_root!(InMemoryRoot);

/// A root kept in a file readable only by its owner (mode 0600), for machines without a
/// keychain: headless servers, containers, tests. The file holds the 32 key bytes.
#[derive(Debug, Clone)]
pub struct KeyFileRoot {
    local: LocalRoot,
    path: PathBuf,
}

impl KeyFileRoot {
    /// Reads the key file at `path`, creating it with a new random key if it does not exist.
    /// A file others can read is refused rather than trusted. This reads the disk; call it
    /// off the async runtime.
    pub fn open_or_create(path: impl AsRef<Path>) -> Result<Self, KeyError> {
        let path = path.as_ref().to_owned();
        let failed = |e: std::io::Error| KeyError::Root(format!("{}: {e}", path.display()));
        let key = match create(&path) {
            Ok(key) => Ok(key),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => read(&path).and_then(|bytes| {
                KeyMaterial::from_bytes(&bytes).map_err(|_| {
                    std::io::Error::new(ErrorKind::InvalidData, "a key file holds 32 bytes")
                })
            }),
            Err(e) => Err(e),
        }
        .map_err(failed)?;
        Ok(Self {
            local: LocalRoot::new(key),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

local_root!(KeyFileRoot);

/// Creates the key file with a new key, failing if it already exists.
fn create(path: &Path) -> std::io::Result<KeyMaterial> {
    let key = KeyMaterial::random().map_err(std::io::Error::other)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(key.bytes())?;
    file.sync_all()?;
    Ok(key)
}

/// Reads an existing key file, refusing one that anyone but its owner can read or write.
fn read(path: &Path) -> std::io::Result<zeroize::Zeroizing<Vec<u8>>> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = file.metadata()?.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(std::io::Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "the key file is open to others (mode {:o}); make it 0600",
                    mode & 0o777
                ),
            ));
        }
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(32));
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use glave_ports::{block_on, file};
    use uuid::Uuid;

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("glave-keys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_key_file_keeps_its_key_across_opens() {
        let path = folder("reopen").join("root.key");
        let org = OrgId::from_uuid(Uuid::from_u128(1));
        let key = KeyMaterial::random().unwrap();
        let first = KeyFileRoot::open_or_create(&path).unwrap();
        let wrapped = block_on!(first.wrap(org, &key)).unwrap();
        let again = KeyFileRoot::open_or_create(&path).unwrap();
        let unwrapped = block_on!(again.unwrap(org, &wrapped)).unwrap();
        assert_eq!(unwrapped.bytes(), key.bytes());
        let other = OrgId::from_uuid(Uuid::from_u128(2));
        assert!(
            block_on!(again.unwrap(other, &wrapped)).is_err(),
            "a wrapped key is bound to its organization"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_others_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let path = folder("open").join("root.key");
        drop(KeyFileRoot::open_or_create(&path).unwrap());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let refused = KeyFileRoot::open_or_create(&path);
        assert!(matches!(refused, Err(KeyError::Root(message)) if message.contains("0600")));
    }

    #[test]
    fn a_key_file_whose_creation_stopped_at_any_step_opens_cleanly() {
        let org = OrgId::from_uuid(Uuid::from_u128(1));
        let key = KeyMaterial::random().unwrap();
        // Stopped before it was put in place: there is no key file yet, and a whole one is made.
        let path = folder("stopped-written").join("root.key");
        file::create_until(&path, key.bytes(), file::Stop::Written).unwrap();
        let made = KeyFileRoot::open_or_create(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap().len(), 32);
        let wrapped = block_on!(made.wrap(org, &key)).unwrap();
        let again = KeyFileRoot::open_or_create(&path).unwrap();
        assert!(block_on!(again.unwrap(org, &wrapped)).is_ok());
        // Stopped once in place, its folder not yet flushed: the whole key is there.
        let path = folder("stopped-placed").join("root.key");
        file::create_until(&path, key.bytes(), file::Stop::Placed).unwrap();
        let opened = KeyFileRoot::open_or_create(&path).unwrap();
        let wrapped = block_on!(opened.wrap(org, &key)).unwrap();
        let planted = LocalRoot::new(KeyMaterial::from_bytes(key.bytes()).unwrap());
        assert_eq!(planted.unwrap(org, &wrapped).unwrap().bytes(), key.bytes());
    }

    #[test]
    fn a_memory_root_cannot_unwrap_another_roots_keys() {
        let org = OrgId::from_uuid(Uuid::from_u128(1));
        let key = KeyMaterial::random().unwrap();
        let wrapped = block_on!(InMemoryRoot::new().unwrap().wrap(org, &key)).unwrap();
        assert!(block_on!(InMemoryRoot::new().unwrap().unwrap(org, &wrapped)).is_err());
    }
}
