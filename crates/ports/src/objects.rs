//! The objects port: where blobs' sealed bytes are kept, apart from their records
//! (`specs/storage.md`, "Which stores"): a folder in glave's folder, or an S3-compatible bucket.
//! Each object is named by its scope and a name the store gives it, written once and read whole
//! as a stream, then deleted.

use crate::blob::{BlobError, Chunks};
use crate::scope::{OrgId, Scope};
use crate::tools::BoxFuture;

/// Keeps objects: blobs' sealed bytes, each written once under a name of its own.
pub trait Objects: Send + Sync {
    /// Starts writing `object` in `scope`, a new name. Nothing is there until the writer
    /// finishes.
    fn create(
        &self,
        scope: Scope,
        object: &str,
    ) -> BoxFuture<'_, Result<Box<dyn ObjectWriter>, BlobError>>;

    /// The bytes of `object` in `scope`, read as they are taken; `None` when there is none.
    fn read(
        &self,
        scope: Scope,
        object: &str,
    ) -> BoxFuture<'_, Result<Option<Box<dyn Chunks>>, BlobError>>;

    /// Deletes `object` in `scope`. One that is not there is already deleted.
    fn delete(&self, scope: Scope, object: &str) -> BoxFuture<'_, Result<(), BlobError>>;

    /// Every object in `org`, finished, with when it was written: for the sweep to find bytes
    /// no record names.
    fn list(&self, org: OrgId) -> BoxFuture<'_, Result<Vec<Listed>, BlobError>>;
}

/// An object as a listing shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub scope: Scope,
    pub object: String,
    /// When it was written, in milliseconds since 1970.
    pub written_ms: i64,
}

/// An object being written: its bytes in order, then finished whole or abandoned.
pub trait ObjectWriter: Send {
    /// Writes the next bytes.
    fn write(&mut self, bytes: Vec<u8>) -> BoxFuture<'_, Result<(), BlobError>>;

    /// Makes what was written the object, all at once.
    fn finish(self: Box<Self>) -> BoxFuture<'static, Result<(), BlobError>>;

    /// Abandons what was written: the object is not there.
    fn abort(self: Box<Self>) -> BoxFuture<'static, Result<(), BlobError>>;
}

/// Whether `object` is a name objects may be kept under: 1 to 64 lowercase letters and digits,
/// so it is one part of a path everywhere.
pub fn object_name_ok(object: &str) -> bool {
    (1..=64).contains(&object.len())
        && object
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}
