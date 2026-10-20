//! The blob store on Postgres: each blob's record (under an HMAC of its SHA-256, so the database
//! does not reveal which known content it holds) in the database, its bytes sealed as an object
//! of random name in the deployment's objects (`specs/storage.md`, "Which stores").

use std::future::Future;
use std::sync::Arc;

use deadpool_postgres::Pool;
use glave_event::{BlobRef, Sha256, ThreadId};
use glave_keys::{KeyError, Opened, seal_into};
use glave_ports::{Blob, BlobError, BlobStore, Chunks, Objects, Scope};

use crate::store::{PostgresKeyring, StoreError, connection, database};

/// The blob store in the database and the deployment's objects.
#[derive(Clone)]
pub struct PostgresBlobs {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
    objects: Arc<dyn Objects>,
}

/// A blob's record, as the `blobs` table keeps it.
struct Record {
    media_type: String,
    size: u64,
    key_id: String,
    object: String,
}

impl PostgresBlobs {
    pub(crate) fn new(pool: Pool, keys: Arc<PostgresKeyring>, objects: Arc<dyn Objects>) -> Self {
        Self {
            pool,
            keys,
            objects,
        }
    }

    /// Records `record` under `name` with `thread` referring to it, unless content of the same
    /// name is kept already: then that is kept and `thread` refers to it. The record kept. The
    /// record's row is locked until the reference is in, so a release running at once either
    /// sees the reference or deletes the record first.
    async fn record(
        &self,
        scope: Scope,
        name: &str,
        thread: ThreadId,
        record: Record,
    ) -> Result<Record, BlobError> {
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let mut client = connection(&self.pool).await.map_err(|e| storage(&e))?;
        let transaction = client
            .transaction()
            .await
            .map_err(database(BlobError::Storage))?;
        let size = i64::try_from(record.size).unwrap_or(i64::MAX);
        let row = transaction
            .query_one(
                "INSERT INTO blobs (org, project, name, media_type, size, key_id, object)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (org, project, name) DO UPDATE SET name = blobs.name
                 RETURNING object, media_type, size, key_id",
                &[
                    &org,
                    &project,
                    &name,
                    &record.media_type,
                    &size,
                    &record.key_id,
                    &record.object,
                ],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        let size: i64 = row.try_get(2).map_err(database(BlobError::Storage))?;
        let kept = Record {
            object: row.try_get(0).map_err(database(BlobError::Storage))?,
            media_type: row.try_get(1).map_err(database(BlobError::Storage))?,
            size: u64::try_from(size).unwrap_or_default(),
            key_id: row.try_get(3).map_err(database(BlobError::Storage))?,
        };
        transaction
            .execute(
                "INSERT INTO blob_threads (org, project, name, thread)
                 VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                &[&org, &project, &name, &thread.as_uuid()],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        transaction
            .commit()
            .await
            .map_err(database(BlobError::Storage))?;
        Ok(kept)
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
        thread: ThreadId,
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
        let kept = self.record(scope, &name, thread, record).await;
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
        let client = connection(&self.pool).await.map_err(|e| storage(&e))?;
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
        let record = Record {
            media_type: row.try_get(0).map_err(database(BlobError::Storage))?,
            size: u64::try_from(size).unwrap_or_default(),
            key_id: row.try_get(2).map_err(database(BlobError::Storage))?,
            object: row.try_get(3).map_err(database(BlobError::Storage))?,
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

    async fn release_now(&self, scope: Scope, thread: ThreadId) -> Result<(), BlobError> {
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let mut client = connection(&self.pool).await.map_err(|e| storage(&e))?;
        let transaction = client
            .transaction()
            .await
            .map_err(database(BlobError::Storage))?;
        let names: Vec<String> = transaction
            .query(
                "DELETE FROM blob_threads WHERE org = $1 AND project = $2 AND thread = $3
                 RETURNING name",
                &[&org, &project, &thread.as_uuid()],
            )
            .await
            .map_err(database(BlobError::Storage))?
            .iter()
            .map(|row| row.try_get(0))
            .collect::<Result<_, _>>()
            .map_err(database(BlobError::Storage))?;
        // Each record is locked first, so a put referring to it at once has either committed
        // its reference, which the delete then sees, or waits and makes the record anew.
        transaction
            .execute(
                "SELECT name FROM blobs WHERE org = $1 AND project = $2 AND name = ANY($3)
                 ORDER BY name FOR UPDATE",
                &[&org, &project, &names],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        let objects: Vec<String> = transaction
            .query(
                "DELETE FROM blobs AS b WHERE b.org = $1 AND b.project = $2 AND b.name = ANY($3)
                 AND NOT EXISTS (SELECT 1 FROM blob_threads AS t
                     WHERE t.org = b.org AND t.project = b.project AND t.name = b.name)
                 RETURNING object",
                &[&org, &project, &names],
            )
            .await
            .map_err(database(BlobError::Storage))?
            .iter()
            .map(|row| row.try_get(0))
            .collect::<Result<_, _>>()
            .map_err(database(BlobError::Storage))?;
        transaction
            .commit()
            .await
            .map_err(database(BlobError::Storage))?;
        drop(client);
        for object in &objects {
            self.forget(scope, object).await;
        }
        Ok(())
    }
}

impl BlobStore for PostgresBlobs {
    fn put(
        &self,
        scope: &Scope,
        thread: ThreadId,
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

    fn release(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<(), BlobError>> + Send {
        self.release_now(*scope, thread)
    }
}

fn storage(error: &StoreError) -> BlobError {
    BlobError::Storage(error.to_string())
}

fn sealing(error: &KeyError) -> BlobError {
    BlobError::Storage(format!("a blob could not be sealed or opened: {error}"))
}
