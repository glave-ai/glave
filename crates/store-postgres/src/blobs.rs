//! The blob store on Postgres: each blob's record (under an HMAC of its SHA-256, so the database
//! does not reveal which known content it holds) in the database, its bytes sealed as an object
//! of random name in the deployment's objects (`specs/storage.md`, "Which stores"); kept while a
//! thread refers to it, then swept.

mod sweep;

use std::future::Future;
use std::sync::Arc;

use deadpool_postgres::Pool;
use glave_event::ThreadId;
use glave_keys::{BlobRecord, BlobRecords, SealedBlobs};
use glave_ports::{BlobError, Clock, Objects, OrgId, Scope, Swept};

use crate::keys::PostgresKeys;
use crate::store::{PostgresKeyring, StoreError, connection, database};

/// The blob store in the database and the deployment's objects.
pub type PostgresBlobs = SealedBlobs<PostgresRecords>;

/// The blob store's records in the database, and the keys and objects beside them.
#[derive(Clone)]
pub struct PostgresRecords {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
    objects: Arc<dyn Objects>,
    clock: Arc<dyn Clock>,
}

impl PostgresRecords {
    pub(crate) fn new(
        pool: Pool,
        keys: Arc<PostgresKeyring>,
        objects: Arc<dyn Objects>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            pool,
            keys,
            objects,
            clock,
        }
    }
}

impl BlobRecords for PostgresRecords {
    type Storage = PostgresKeys;

    fn keys(&self) -> &PostgresKeyring {
        &self.keys
    }

    fn objects(&self) -> &dyn Objects {
        self.objects.as_ref()
    }

    /// The record's row is locked until the reference is in, so a sweep running at once
    /// either sees the reference or deletes the record first.
    async fn record(
        &self,
        scope: Scope,
        name: String,
        thread: Option<ThreadId>,
        record: BlobRecord,
    ) -> Result<BlobRecord, BlobError> {
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let now = self.clock.now().as_millisecond();
        let mut client = connection(&self.pool, scope.org())
            .await
            .map_err(|e| storage(&e))?;
        let transaction = client
            .transaction()
            .await
            .map_err(database(BlobError::Storage))?;
        let size = i64::try_from(record.size).unwrap_or(i64::MAX);
        let row = transaction
            .query_one(
                "INSERT INTO blobs
                     (org, project, name, media_type, size, key_id, object, referred, uploaded)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (org, project, name) DO UPDATE
                 SET referred = excluded.referred,
                     uploaded = CASE WHEN excluded.referred THEN blobs.uploaded
                                     ELSE excluded.uploaded END
                 RETURNING object, media_type, size, key_id",
                &[
                    &org,
                    &project,
                    &name,
                    &record.media_type,
                    &size,
                    &record.key_id,
                    &record.object,
                    &thread.is_some(),
                    &now,
                ],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        let size: i64 = row.try_get(2).map_err(database(BlobError::Storage))?;
        let kept = BlobRecord {
            object: row.try_get(0).map_err(database(BlobError::Storage))?,
            media_type: row.try_get(1).map_err(database(BlobError::Storage))?,
            size: u64::try_from(size).unwrap_or_default(),
            key_id: row.try_get(3).map_err(database(BlobError::Storage))?,
        };
        if let Some(thread) = thread {
            transaction
                .execute(
                    "INSERT INTO blob_threads (org, project, name, thread)
                     VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                    &[&org, &project, &name, &thread.as_uuid()],
                )
                .await
                .map_err(database(BlobError::Storage))?;
        }
        transaction
            .commit()
            .await
            .map_err(database(BlobError::Storage))?;
        Ok(kept)
    }

    async fn find(&self, scope: Scope, name: String) -> Result<Option<BlobRecord>, BlobError> {
        let client = connection(&self.pool, scope.org())
            .await
            .map_err(|e| storage(&e))?;
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let row = client
            .query_opt(
                "SELECT media_type, size, key_id, object FROM blobs
                 WHERE org = $1 AND project = $2 AND name = $3",
                &[&org, &project, &name],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        drop(client);
        let Some(row) = row else {
            return Ok(None);
        };
        let size: i64 = row.try_get(1).map_err(database(BlobError::Storage))?;
        Ok(Some(BlobRecord {
            media_type: row.try_get(0).map_err(database(BlobError::Storage))?,
            size: u64::try_from(size).unwrap_or_default(),
            key_id: row.try_get(2).map_err(database(BlobError::Storage))?,
            object: row.try_get(3).map_err(database(BlobError::Storage))?,
        }))
    }
    /// The record's row is locked by the update, so a sweep that locked it first has deleted
    /// it by the time this reads it, and one that comes after sees the reference.
    async fn refer(&self, scope: Scope, name: String, thread: ThreadId) -> Result<bool, BlobError> {
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let mut client = connection(&self.pool, scope.org())
            .await
            .map_err(|e| storage(&e))?;
        let transaction = client
            .transaction()
            .await
            .map_err(database(BlobError::Storage))?;
        let found = transaction
            .execute(
                "UPDATE blobs SET referred = true WHERE org = $1 AND project = $2 AND name = $3",
                &[&org, &project, &name],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        if found == 1 {
            transaction
                .execute(
                    "INSERT INTO blob_threads (org, project, name, thread)
                     VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                    &[&org, &project, &name, &thread.as_uuid()],
                )
                .await
                .map_err(database(BlobError::Storage))?;
        }
        transaction
            .commit()
            .await
            .map_err(database(BlobError::Storage))?;
        Ok(found == 1)
    }

    async fn release(&self, scope: Scope, thread: ThreadId) -> Result<(), BlobError> {
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let client = connection(&self.pool, scope.org())
            .await
            .map_err(|e| storage(&e))?;
        client
            .execute(
                "DELETE FROM blob_threads WHERE org = $1 AND project = $2 AND thread = $3",
                &[&org, &project, &thread.as_uuid()],
            )
            .await
            .map(drop)
            .map_err(database(BlobError::Storage))
    }

    fn sweep(&self, org: OrgId) -> impl Future<Output = Result<Swept, BlobError>> + Send {
        self.sweep_now(org)
    }
}

fn storage(error: &StoreError) -> BlobError {
    BlobError::Storage(error.to_string())
}
