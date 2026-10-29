//! The directory (`specs/storage.md`, "Sign-in is the one lookup before an organization is
//! known"): from an identity a person signs in with to the person, and from the person to the
//! organizations they belong to, in the order they joined. It holds ids and the hashes its
//! callers make of identities, never an identity itself and nothing of any organization's, so
//! it is read before anyone's organization is known.

use glave_event::PrincipalId;
use thiserror::Error;

use crate::scope::OrgId;
use crate::tools::BoxFuture;

/// An identity as the directory keeps it: a SHA-256 in lowercase hex its caller made of it
/// (a company login's issuer and subject, a passkey's user handle, an email), never the value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentityKey(String);

impl IdentityKey {
    /// `hex` as a key: refused unless it is 64 lowercase hex digits.
    pub fn new(hex: &str) -> Result<Self, DirectoryError> {
        let digest = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if digest {
            Ok(Self(hex.to_owned()))
        } else {
            Err(DirectoryError::Key)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a directory call failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DirectoryError {
    #[error("an identity's key must be a SHA-256 in lowercase hex")]
    Key,
    #[error("directory storage failed: {0}")]
    Storage(String),
}

/// Where the directory is kept: the data store, beside every organization's data and in none.
pub trait Directory: Send + Sync {
    /// Links `identity` to `person`, unless it leads to someone already: whom it leads to now.
    /// Of callers racing to link one identity, one wins and every one is told whom.
    fn link<'a>(
        &'a self,
        identity: &'a IdentityKey,
        person: PrincipalId,
    ) -> BoxFuture<'a, Result<PrincipalId, DirectoryError>>;

    /// Whom `identity` leads to, if anyone.
    fn find<'a>(
        &'a self,
        identity: &'a IdentityKey,
    ) -> BoxFuture<'a, Result<Option<PrincipalId>, DirectoryError>>;

    /// Records that `person` belongs to `org`, after every organization they joined before;
    /// joining one they belong to changes nothing.
    fn join(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>>;

    /// Takes `person` out of `org`; one they do not belong to changes nothing.
    fn leave(&self, person: PrincipalId, org: OrgId) -> BoxFuture<'_, Result<(), DirectoryError>>;

    /// The organizations `person` belongs to, the first they joined first.
    fn organizations(
        &self,
        person: PrincipalId,
    ) -> BoxFuture<'_, Result<Vec<OrgId>, DirectoryError>>;
}
