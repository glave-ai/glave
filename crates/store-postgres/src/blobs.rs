//! The blob store on Postgres: each blob's record (under an HMAC of its SHA-256, so the database
//! does not reveal which known content it holds) apart from its bytes, sealed a chunk at a time
//! as an object of random name.

use std::future::Future;
use std::sync::Arc;

use deadpool_postgres::{GenericClient, Pool};
use glave_event::{BlobRef, Sha256, ThreadId};
use glave_keys::{BlobOpener, BlobSealer, Finished, KeyError, chunks_in};
use glave_ports::{Blob, BlobError, BlobStore, BoxFuture, Chunks, Scope};
use uuid::Uuid;

use crate::store::{PostgresKeyring, StoreError, connection, database};

/// The blob store in the database.
#[derive(Clone)]
pub struct PostgresBlobs {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
}

/// A blob's record, as the `blobs` table keeps it.
struct Record {
    media_type: String,
    size: u64,
    key_id: String,
    object: String,
}

impl PostgresBlobs {
    pub(crate) fn new(pool: Pool, keys: Arc<PostgresKeyring>) -> Self {
        Self { pool, keys }
    }

    /// Seals what `bytes` yields into `sealer`'s object, a chunk at a time, refusing more than
    /// `limit` bytes.
    async fn write(
        &self,
        scope: Scope,
        mut sealer: BlobSealer,
        limit: u64,
        bytes: &mut dyn Chunks,
    ) -> Result<Finished, BlobError> {
        let object = sealer.object().to_owned();
        let client = connection(&self.pool).await.map_err(|e| storage(&e))?;
        let insert = client
            .prepare_cached(
                "INSERT INTO blob_chunks (org, project, object, n, bytes)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .await
            .map_err(database(BlobError::Storage))?;
        let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
        let mut n = 0_i64;
        while let Some(piece) = bytes.next_chunk().await? {
            for chunk in sealer.write(&piece).map_err(|e| sealing(&e))? {
                client
                    .execute(&insert, &[&org, &project, &object, &n, &chunk])
                    .await
                    .map_err(database(BlobError::Storage))?;
                n += 1;
            }
            if sealer.size() > limit {
                return Err(BlobError::TooLarge(limit));
            }
        }
        let finished = sealer.finish().map_err(|e| sealing(&e))?;
        client
            .execute(&insert, &[&org, &project, &object, &n, &finished.last])
            .await
            .map_err(database(BlobError::Storage))?;
        Ok(finished)
    }

    /// Records `record` under `name` with `thread` referring to it, unless content of the same
    /// name is kept already: then that is kept, `thread` refers to it, and this object goes.
    /// The record kept. The record's row is locked until the reference is in, so a release
    /// running at once either sees the reference or deletes the record first.
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
                 RETURNING object, media_type, size",
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
        let object: String = row.try_get(0).map_err(database(BlobError::Storage))?;
        let media_type: String = row.try_get(1).map_err(database(BlobError::Storage))?;
        let size: i64 = row.try_get(2).map_err(database(BlobError::Storage))?;
        transaction
            .execute(
                "INSERT INTO blob_threads (org, project, name, thread)
                 VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                &[&org, &project, &name, &thread.as_uuid()],
            )
            .await
            .map_err(database(BlobError::Storage))?;
        if object != record.object {
            drop_object(&transaction, org, project, &record.object).await?;
        }
        transaction
            .commit()
            .await
            .map_err(database(BlobError::Storage))?;
        Ok(Record {
            media_type,
            size: u64::try_from(size).unwrap_or_default(),
            key_id: record.key_id,
            object,
        })
    }

    /// Deletes an object no record names: one a refused or failed write left.
    async fn forget(&self, scope: Scope, object: &str) {
        let dropped = match connection(&self.pool).await {
            Ok(client) => {
                let (org, project) = (scope.org().as_uuid(), scope.project().as_uuid());
                drop_object(&client, org, project, object).await
            }
            Err(error) => Err(storage(&error)),
        };
        if let Err(error) = dropped {
            tracing::warn!(%error, "an unfinished blob could not be removed");
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
        let written = self.write(scope, sealer, limit, bytes.as_mut()).await;
        let finished = match written {
            Ok(finished) => finished,
            Err(error) => {
                self.forget(scope, &object).await;
                return Err(error);
            }
        };
        let name = self.keys.blob_name(&scope, &finished.sha256).await;
        let Ok(Some(name)) = name else {
            self.forget(scope, &object).await;
            return Err(BlobError::Storage(format!(
                "the project's key is gone, so a blob cannot be named: {name:?}"
            )));
        };
        let record = Record {
            media_type,
            size: finished.size,
            key_id,
            object: object.clone(),
        };
        let kept = match self.record(scope, &name, thread, record).await {
            Ok(kept) => kept,
            Err(error) => {
                self.forget(scope, &object).await;
                return Err(error);
            }
        };
        Ok(BlobRef {
            sha256: finished.sha256,
            bytes: kept.size,
            media_type: kept.media_type,
        })
    }

    async fn get_now(&self, scope: Scope, sha256: Sha256) -> Result<Option<Blob>, BlobError> {
        // A project with no key yet (or an erased one) has nothing that can be found.
        let Some(name) = self
            .keys
            .blob_name(&scope, &sha256)
            .await
            .map_err(|e| sealing(&e))?
        else {
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
        Ok(Some(Blob {
            reference: BlobRef {
                sha256,
                bytes: record.size,
                media_type: record.media_type,
            },
            bytes: Box::new(Stored {
                pool: self.pool.clone(),
                scope,
                object: record.object,
                n: 0,
                count: chunks_in(record.size),
                opener,
            }),
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
        for object in &objects {
            drop_object(&transaction, org, project, object).await?;
        }
        transaction
            .commit()
            .await
            .map_err(database(BlobError::Storage))
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

/// Deletes the chunks of `object`.
async fn drop_object<C: GenericClient>(
    client: &C,
    org: Uuid,
    project: Uuid,
    object: &str,
) -> Result<(), BlobError> {
    client
        .execute(
            "DELETE FROM blob_chunks WHERE org = $1 AND project = $2 AND object = $3",
            &[&org, &project, &object],
        )
        .await
        .map(drop)
        .map_err(database(BlobError::Storage))
}

/// A blob's bytes, read and opened a chunk at a time.
struct Stored {
    pool: Pool,
    scope: Scope,
    object: String,
    n: u64,
    count: u64,
    opener: BlobOpener,
}

impl Stored {
    async fn next_now(&mut self) -> Result<Option<Vec<u8>>, BlobError> {
        if self.n >= self.count {
            return Ok(None);
        }
        let n = i64::try_from(self.n).map_err(|e| BlobError::Storage(e.to_string()))?;
        let (org, project) = (self.scope.org().as_uuid(), self.scope.project().as_uuid());
        let client = connection(&self.pool).await.map_err(|e| storage(&e))?;
        let row = client
            .query_opt(
                "SELECT bytes FROM blob_chunks
                 WHERE org = $1 AND project = $2 AND object = $3 AND n = $4",
                &[&org, &project, &self.object, &n],
            )
            .await
            .map_err(database(BlobError::Storage))?
            .ok_or_else(|| BlobError::Storage(format!("chunk {n} of a blob is missing")))?;
        let sealed: Vec<u8> = row.try_get(0).map_err(database(BlobError::Storage))?;
        self.n += 1;
        let last = self.n == self.count;
        self.opener
            .open(&sealed, last)
            .map(Some)
            .map_err(|e| sealing(&e))
    }
}

impl Chunks for Stored {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>> {
        Box::pin(self.next_now())
    }
}

fn storage(error: &StoreError) -> BlobError {
    BlobError::Storage(error.to_string())
}

fn sealing(error: &KeyError) -> BlobError {
    BlobError::Storage(format!("a blob could not be sealed or opened: {error}"))
}
