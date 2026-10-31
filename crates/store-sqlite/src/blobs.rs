//! The blob store on SQLite: each blob's record (under an HMAC of its SHA-256, so the file does
//! not reveal which known content it holds) in the file, its bytes sealed as an object of random
//! name in the deployment's objects (`specs/storage.md`, "Which stores"); kept while a thread
//! refers to it, then swept.

mod sweep;

use std::future::Future;
use std::sync::Arc;

use glave_event::{BlobRef, Sha256, ThreadId};
use glave_keys::{KeyError, Opened, seal_into};
use glave_ports::{Blob, BlobError, BlobStore, Chunks, Clock, Objects, OrgId, Scope, Swept};
use rusqlite::{OptionalExtension as _, params};

use crate::store::{Shared, SqliteKeyring, StoreError, run};

/// The blob store in the local database and the objects beside it.
#[derive(Clone)]
pub struct SqliteBlobs {
    connection: Shared,
    keys: Arc<SqliteKeyring>,
    objects: Arc<dyn Objects>,
    clock: Arc<dyn Clock>,
}

/// A blob's record, as the `blobs` table keeps it.
struct Record {
    media_type: String,
    size: u64,
    key_id: String,
    object: String,
}

impl SqliteBlobs {
    pub(crate) fn new(
        connection: Shared,
        keys: Arc<SqliteKeyring>,
        objects: Arc<dyn Objects>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            connection,
            keys,
            objects,
            clock,
        }
    }

    /// Records `record` under `name`, `thread` referring to it when given, unless content of
    /// the same name is kept already: then that is kept and `thread` refers to it. An upload
    /// (no thread) is kept a day from now, as one no thread referred to since, even when one
    /// did before. The record kept.
    async fn record(
        &self,
        scope: Scope,
        name: String,
        thread: Option<ThreadId>,
        record: Record,
    ) -> Result<Record, BlobError> {
        let now = self.clock.now().as_millisecond();
        run(&self.connection, move |connection| {
            let (org, project) = (scope.org().to_string(), scope.project().to_string());
            let transaction = connection.transaction()?;
            let size = i64::try_from(record.size).unwrap_or(i64::MAX);
            let (object, media_type, size, key_id): (String, String, i64, String) = transaction
                .query_row(
                    "INSERT INTO blobs
                         (org, project, name, media_type, size, key_id, object, referred, uploaded)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT (org, project, name) DO UPDATE
                     SET referred = excluded.referred,
                         uploaded = CASE WHEN excluded.referred = 1 THEN blobs.uploaded
                                         ELSE excluded.uploaded END
                     RETURNING object, media_type, size, key_id",
                    params![
                        org,
                        project,
                        name,
                        record.media_type,
                        size,
                        record.key_id,
                        record.object,
                        thread.is_some(),
                        now
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )?;
            if let Some(thread) = thread {
                transaction.execute(
                    "INSERT OR IGNORE INTO blob_threads (org, project, name, thread)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![org, project, name, thread.to_string()],
                )?;
            }
            transaction.commit()?;
            Ok(Record {
                media_type,
                size: u64::try_from(size).unwrap_or_default(),
                key_id,
                object,
            })
        })
        .await
        .map_err(|e| storage(&e))
    }

    /// Deletes an object no record names: one a refused or failed write left, a second copy, or
    /// one whose record went.
    async fn forget(&self, scope: Scope, object: &str) {
        if let Err(error) = self.objects.delete(scope, object).await {
            tracing::warn!(%error, "a blob's bytes could not be removed");
        }
    }

    async fn put_now(
        &self,
        scope: Scope,
        thread: Option<ThreadId>,
        media_type: String,
        limit: u64,
        mut bytes: Box<dyn Chunks>,
    ) -> Result<BlobRef, BlobError> {
        let sealer = self
            .keys
            .blob_sealer(&scope)
            .await
            .map_err(|e| sealing(&e))?;
        let (object, key_id) = (sealer.object().to_owned(), sealer.key_id().to_owned());
        let mut writer = self.objects.create(scope, &object).await?;
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
        let name = match self.keys.blob_name(&scope, &finished.sha256).await {
            Ok(Some(name)) => name,
            named => {
                self.forget(scope, &object).await;
                return Err(BlobError::Storage(format!(
                    "the project's key is gone, so a blob cannot be named: {named:?}"
                )));
            }
        };
        let record = Record {
            media_type,
            size: finished.size,
            key_id,
            object: object.clone(),
        };
        let kept = self.record(scope, name, thread, record).await;
        if kept.as_ref().map_or(true, |kept| kept.object != object) {
            self.forget(scope, &object).await;
        }
        let kept = kept?;
        Ok(BlobRef {
            sha256: finished.sha256,
            bytes: kept.size,
            media_type: kept.media_type,
        })
    }

    async fn get_now(&self, scope: Scope, sha256: Sha256) -> Result<Option<Blob>, BlobError> {
        // A project with no key yet (or an erased one) has nothing that can be found.
        let name = self
            .keys
            .blob_name(&scope, &sha256)
            .await
            .map_err(|e| sealing(&e))?;
        let Some(name) = name else {
            return Ok(None);
        };
        let record = run(&self.connection, move |connection| {
            connection
                .query_row(
                    "SELECT media_type, size, key_id, object FROM blobs
                     WHERE org = ?1 AND project = ?2 AND name = ?3",
                    params![scope.org().to_string(), scope.project().to_string(), name],
                    |row| {
                        let size: i64 = row.get(1)?;
                        Ok(Record {
                            media_type: row.get(0)?,
                            size: u64::try_from(size).unwrap_or_default(),
                            key_id: row.get(2)?,
                            object: row.get(3)?,
                        })
                    },
                )
                .optional()
        })
        .await
        .map_err(|e| storage(&e))?;
        let Some(record) = record else {
            return Ok(None);
        };
        let opener = self
            .keys
            .blob_opener(&scope, &record.key_id, &record.object, &sha256)
            .await
            .map_err(|e| sealing(&e))?;
        let sealed = self
            .objects
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

    async fn refer_now(
        &self,
        scope: Scope,
        sha256: Sha256,
        thread: ThreadId,
    ) -> Result<bool, BlobError> {
        let name = self
            .keys
            .blob_name(&scope, &sha256)
            .await
            .map_err(|e| sealing(&e))?;
        let Some(name) = name else {
            return Ok(false);
        };
        run(&self.connection, move |connection| {
            let (org, project) = (scope.org().to_string(), scope.project().to_string());
            let transaction = connection.transaction()?;
            let found = transaction.execute(
                "UPDATE blobs SET referred = 1 WHERE org = ?1 AND project = ?2 AND name = ?3",
                params![org, project, name],
            )?;
            if found == 1 {
                transaction.execute(
                    "INSERT OR IGNORE INTO blob_threads (org, project, name, thread)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![org, project, name, thread.to_string()],
                )?;
            }
            transaction.commit()?;
            Ok(found == 1)
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn release_now(&self, scope: Scope, thread: ThreadId) -> Result<(), BlobError> {
        run(&self.connection, move |connection| {
            connection.execute(
                "DELETE FROM blob_threads WHERE org = ?1 AND project = ?2 AND thread = ?3",
                params![
                    scope.org().to_string(),
                    scope.project().to_string(),
                    thread.to_string()
                ],
            )
        })
        .await
        .map(drop)
        .map_err(|e| storage(&e))
    }
}

impl BlobStore for SqliteBlobs {
    fn put(
        &self,
        scope: &Scope,
        thread: Option<ThreadId>,
        media_type: &str,
        limit: u64,
        bytes: Box<dyn Chunks>,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send {
        self.put_now(*scope, thread, media_type.to_owned(), limit, bytes)
    }

    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Blob>, BlobError>> + Send {
        self.get_now(*scope, sha256.clone())
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
        self.release_now(*scope, thread)
    }

    fn sweep(&self, org: OrgId) -> impl Future<Output = Result<Swept, BlobError>> + Send {
        self.sweep_now(org)
    }

    fn organizations(&self) -> impl Future<Output = Result<Vec<OrgId>, BlobError>> + Send {
        self.organizations_now()
    }
}

fn storage(error: &StoreError) -> BlobError {
    BlobError::Storage(error.to_string())
}

fn sealing(error: &KeyError) -> BlobError {
    BlobError::Storage(format!("a blob could not be sealed or opened: {error}"))
}
