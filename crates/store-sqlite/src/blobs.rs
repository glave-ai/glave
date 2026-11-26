//! The blob store on SQLite: each blob's record (under an HMAC of its SHA-256, so the file does
//! not reveal which known content it holds) in the file, its bytes sealed as an object of random
//! name in the deployment's objects (`specs/storage.md`, "Which stores"); kept while a thread
//! refers to it, then swept.

mod sweep;

use std::future::Future;
use std::sync::Arc;

use glave_event::ThreadId;
use glave_keys::{BlobRecord, BlobRecords, SealedBlobs};
use glave_ports::{BlobError, Clock, Objects, OrgId, Scope, Swept};
use rusqlite::{OptionalExtension as _, params};

use crate::keys::SqliteKeys;
use crate::store::{Shared, SqliteKeyring, StoreError, run};

/// The blob store in the local database and the objects beside it.
pub type SqliteBlobs = SealedBlobs<SqliteRecords>;

/// The blob store's records in the local database, and the keys and objects beside them.
#[derive(Clone)]
pub struct SqliteRecords {
    connection: Shared,
    keys: Arc<SqliteKeyring>,
    objects: Arc<dyn Objects>,
    clock: Arc<dyn Clock>,
}

impl SqliteRecords {
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
}

impl BlobRecords for SqliteRecords {
    type Storage = SqliteKeys;

    fn keys(&self) -> &SqliteKeyring {
        &self.keys
    }

    fn objects(&self) -> &dyn Objects {
        self.objects.as_ref()
    }

    async fn record(
        &self,
        scope: Scope,
        name: String,
        thread: Option<ThreadId>,
        record: BlobRecord,
    ) -> Result<BlobRecord, BlobError> {
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
            Ok(BlobRecord {
                media_type,
                size: u64::try_from(size).unwrap_or_default(),
                key_id,
                object,
            })
        })
        .await
        .map_err(|e| storage(&e))
    }

    async fn find(&self, scope: Scope, name: String) -> Result<Option<BlobRecord>, BlobError> {
        run(&self.connection, move |connection| {
            connection
                .query_row(
                    "SELECT media_type, size, key_id, object FROM blobs
                     WHERE org = ?1 AND project = ?2 AND name = ?3",
                    params![scope.org().to_string(), scope.project().to_string(), name],
                    |row| {
                        let size: i64 = row.get(1)?;
                        Ok(BlobRecord {
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
        .map_err(|e| storage(&e))
    }
    async fn refer(&self, scope: Scope, name: String, thread: ThreadId) -> Result<bool, BlobError> {
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

    async fn release(&self, scope: Scope, thread: ThreadId) -> Result<(), BlobError> {
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

    fn sweep(&self, org: OrgId) -> impl Future<Output = Result<Swept, BlobError>> + Send {
        self.sweep_now(org)
    }
}

fn storage(error: &StoreError) -> BlobError {
    BlobError::Storage(error.to_string())
}
