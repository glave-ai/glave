//! A blob store's flow, the same on every database: its bytes sealed into an object of random
//! name, then its record kept under the name the project key gives its hash; read back by that
//! name, opened as it is read. Each store brings only its steps in SQL, through
//! [`BlobRecords`], and [`SealedBlobs`] is its blob store.

use std::future::Future;

use glave_event::{BlobRef, Sha256, ThreadId};
use glave_ports::{Blob, BlobError, BlobStore, Chunks, Objects, OrgId, Scope, Swept};

use crate::storage::KeyStorage;

use super::super::Keys;
use super::{Opened, seal_into, sealing};

/// A blob's record, as a store's `blobs` table keeps it.
pub struct BlobRecord {
    pub media_type: String,
    pub size: u64,
    pub key_id: String,
    pub object: String,
}

/// A blob store's keys, its objects, and its steps in its database.
pub trait BlobRecords: Send + Sync {
    /// Where the store's keys are kept.
    type Storage: KeyStorage;

    /// The keys that seal the store's blobs.
    fn keys(&self) -> &Keys<Self::Storage>;

    /// Where the sealed bytes are kept.
    fn objects(&self) -> &dyn Objects;

    /// Records `record` under `name`, `thread` referring to it when given, unless content of
    /// the same name is kept already: then that is kept and `thread` refers to it. An upload
    /// (no thread) is kept a day from now, as one no thread referred to since, even when one
    /// did before. The record kept.
    fn record(
        &self,
        scope: Scope,
        name: String,
        thread: Option<ThreadId>,
        record: BlobRecord,
    ) -> impl Future<Output = Result<BlobRecord, BlobError>> + Send;

    /// The record kept under `name`, if there is one.
    fn find(
        &self,
        scope: Scope,
        name: String,
    ) -> impl Future<Output = Result<Option<BlobRecord>, BlobError>> + Send;

    /// `thread` refers to the record under `name`; whether there is one.
    fn refer(
        &self,
        scope: Scope,
        name: String,
        thread: ThreadId,
    ) -> impl Future<Output = Result<bool, BlobError>> + Send;

    /// `thread` refers to nothing in `scope` any more.
    fn release(
        &self,
        scope: Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<(), BlobError>> + Send;

    /// Deletes what nothing refers to in `org` (`specs/storage.md`, "Blobs").
    fn sweep(&self, org: OrgId) -> impl Future<Output = Result<Swept, BlobError>> + Send;
}

/// The blob store over a database's [`BlobRecords`]: content sealed into objects of random
/// name, each found by the name its project key gives its hash.
#[derive(Clone)]
pub struct SealedBlobs<R>(R);

impl<R> SealedBlobs<R> {
    pub fn new(records: R) -> Self {
        Self(records)
    }
}

impl<R: BlobRecords> SealedBlobs<R> {
    async fn refer_now(
        &self,
        scope: Scope,
        sha256: Sha256,
        thread: ThreadId,
    ) -> Result<bool, BlobError> {
        let Some(name) = sealed_name(&self.0, &scope, &sha256).await? else {
            return Ok(false);
        };
        self.0.refer(scope, name, thread).await
    }
}

impl<R: BlobRecords> BlobStore for SealedBlobs<R> {
    fn put(
        &self,
        scope: &Scope,
        thread: Option<ThreadId>,
        media_type: &str,
        limit: u64,
        bytes: Box<dyn Chunks>,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send {
        put_sealed(&self.0, *scope, thread, media_type.to_owned(), limit, bytes)
    }

    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Blob>, BlobError>> + Send {
        get_sealed(&self.0, *scope, sha256.clone())
    }

    fn refer(
        &self,
        scope: &Scope,
        sha256: &Sha256,
        thread: ThreadId,
    ) -> impl Future<Output = Result<bool, BlobError>> + Send {
        self.refer_now(*scope, sha256.clone(), thread)
    }

    fn release(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<(), BlobError>> + Send {
        self.0.release(*scope, thread)
    }

    fn sweep(&self, org: OrgId) -> impl Future<Output = Result<Swept, BlobError>> + Send {
        self.0.sweep(org)
    }
}

/// Seals what `bytes` yields as a new object, refusing more than `limit` bytes, and records it,
/// `thread` referring to it when given. Content kept already is kept instead, and the new copy
/// deleted.
async fn put_sealed<R: BlobRecords>(
    store: &R,
    scope: Scope,
    thread: Option<ThreadId>,
    media_type: String,
    limit: u64,
    mut bytes: Box<dyn Chunks>,
) -> Result<BlobRef, BlobError> {
    let sealer = store
        .keys()
        .blob_sealer(&scope)
        .await
        .map_err(|e| sealing(&e))?;
    let (object, key_id) = (sealer.object().to_owned(), sealer.key_id().to_owned());
    let mut writer = store.objects().create(scope, &object).await?;
    let finished = match seal_into(sealer, limit, bytes.as_mut(), writer.as_mut()).await {
        Ok(finished) => finished,
        Err(error) => {
            if let Err(abandoned) = writer.abort().await {
                tracing::warn!(error = %abandoned, "an unfinished blob could not be abandoned");
            }
            return Err(error);
        }
    };
    writer.finish().await?;
    let name = match store.keys().blob_name(&scope, &finished.sha256).await {
        Ok(Some(name)) => name,
        named => {
            forget_sealed(store, scope, &object).await;
            return Err(BlobError::Storage(format!(
                "the project's key is gone, so a blob cannot be named: {named:?}"
            )));
        }
    };
    let record = BlobRecord {
        media_type,
        size: finished.size,
        key_id,
        object: object.clone(),
    };
    let kept = store.record(scope, name, thread, record).await;
    if kept.as_ref().map_or(true, |kept| kept.object != object) {
        forget_sealed(store, scope, &object).await;
    }
    let kept = kept?;
    Ok(BlobRef {
        sha256: finished.sha256,
        bytes: kept.size,
        media_type: kept.media_type,
    })
}

/// The blob in `scope` whose content hashes to `sha256`, opened as it is read; `None` when
/// there is none.
async fn get_sealed<R: BlobRecords>(
    store: &R,
    scope: Scope,
    sha256: Sha256,
) -> Result<Option<Blob>, BlobError> {
    // A project with no key yet (or an erased one) has nothing that can be found.
    let Some(name) = sealed_name(store, &scope, &sha256).await? else {
        return Ok(None);
    };
    let Some(record) = store.find(scope, name).await? else {
        return Ok(None);
    };
    let opener = store
        .keys()
        .blob_opener(&scope, &record.key_id, &record.object, &sha256)
        .await
        .map_err(|e| sealing(&e))?;
    let sealed = store
        .objects()
        .read(scope, &record.object)
        .await?
        .ok_or_else(|| BlobError::Storage("a blob's bytes are missing".into()))?;
    Ok(Some(Blob {
        reference: BlobRef {
            sha256,
            bytes: record.size,
            media_type: record.media_type,
        },
        bytes: Box::new(Opened::new(sealed, opener)),
    }))
}

/// The name a blob whose content hashes to `sha256` is recorded under in `scope`; `None` when
/// the project has no key (yet, or any more).
async fn sealed_name<R: BlobRecords>(
    store: &R,
    scope: &Scope,
    sha256: &Sha256,
) -> Result<Option<String>, BlobError> {
    store
        .keys()
        .blob_name(scope, sha256)
        .await
        .map_err(|e| sealing(&e))
}

/// Deletes an object no record names: one a refused or failed write left, a second copy, or
/// one whose record went.
pub async fn forget_sealed<R: BlobRecords>(store: &R, scope: Scope, object: &str) {
    if let Err(error) = store.objects().delete(scope, object).await {
        tracing::warn!(%error, "a blob's bytes could not be removed");
    }
}
