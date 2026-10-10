//! The blob store on Postgres: content kept once per scope, sealed, and stored under an HMAC
//! of its SHA-256 so the database does not reveal which known content it holds.

use std::future::Future;
use std::sync::Arc;

use deadpool_postgres::Pool;
use glave_event::{BlobRef, Sha256, ThreadId};
use glave_ports::{BlobError, BlobStore, Scope, digest};

use crate::store::{PostgresKeyring, connection, describe};

/// The blob store in the database.
#[derive(Clone)]
pub struct PostgresBlobs {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
}

impl PostgresBlobs {
    pub(crate) fn new(pool: Pool, keys: Arc<PostgresKeyring>) -> Self {
        Self { pool, keys }
    }
}

impl BlobStore for PostgresBlobs {
    fn put(
        &self,
        scope: &Scope,
        thread: ThreadId,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send {
        let scope = *scope;
        let media_type = media_type.to_owned();
        let this = self.clone();
        async move {
            let sha256 = digest(&bytes);
            let size = u64::try_from(bytes.len()).map_err(|e| BlobError::Storage(e.to_string()))?;
            let (name, sealed) = this
                .keys
                .seal_blob(&scope, &sha256, &bytes)
                .await
                .map_err(|e| BlobError::Storage(format!("could not seal a blob: {e}")))?;
            let (org, space) = (scope.org.as_uuid(), scope.space.as_uuid());
            let mut client = connection(&this.pool)
                .await
                .map_err(|e| BlobError::Storage(e.to_string()))?;
            let transaction = client.transaction().await.map_err(database)?;
            // Storing it again re-seals it under this month's key.
            transaction
                .execute(
                    "INSERT INTO blobs (org, space, name, media_type, bytes, key_id)
                     VALUES ($1, $2, $3, $4, $5, $6)
                     ON CONFLICT (org, space, name) DO UPDATE
                     SET media_type = excluded.media_type, bytes = excluded.bytes,
                         key_id = excluded.key_id",
                    &[
                        &org,
                        &space,
                        &name,
                        &media_type,
                        &sealed.bytes,
                        &sealed.key_id,
                    ],
                )
                .await
                .map_err(database)?;
            transaction
                .execute(
                    "INSERT INTO blob_threads (org, space, name, thread)
                     VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                    &[&org, &space, &name, &thread.as_uuid()],
                )
                .await
                .map_err(database)?;
            transaction.commit().await.map_err(database)?;
            Ok(BlobRef {
                sha256,
                bytes: size,
                media_type,
            })
        }
    }

    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, BlobError>> + Send {
        let scope = *scope;
        let sha256 = sha256.clone();
        let this = self.clone();
        async move {
            let name = this
                .keys
                .blob_name(&scope, &sha256)
                .await
                .map_err(|e| BlobError::Storage(e.to_string()))?;
            let client = connection(&this.pool)
                .await
                .map_err(|e| BlobError::Storage(e.to_string()))?;
            let (org, space) = (scope.org.as_uuid(), scope.space.as_uuid());
            if let Some(name) = name {
                let row = client
                    .query_opt(
                        "SELECT bytes, key_id FROM blobs
                         WHERE org = $1 AND space = $2 AND name = $3 AND key_id IS NOT NULL",
                        &[&org, &space, &name],
                    )
                    .await
                    .map_err(database)?;
                if let Some(row) = row {
                    let bytes: Vec<u8> = row.try_get(0).map_err(database)?;
                    let key_id: String = row.try_get(1).map_err(database)?;
                    return this
                        .keys
                        .open_blob(&scope, &name, &key_id, &bytes)
                        .await
                        .map(Some)
                        .map_err(|e| {
                            BlobError::Storage(format!("a blob could not be opened: {e}"))
                        });
                }
            }
            // Written before sealing: stored as written, under its hash.
            let row = client
                .query_opt(
                    "SELECT bytes FROM blobs
                     WHERE org = $1 AND space = $2 AND name = $3 AND key_id IS NULL",
                    &[&org, &space, &sha256.as_str()],
                )
                .await
                .map_err(database)?;
            row.map(|row| row.try_get(0)).transpose().map_err(database)
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "taken by value to pass straight to map_err"
)]
fn database(error: tokio_postgres::Error) -> BlobError {
    BlobError::Storage(format!("the database failed: {}", describe(&error)))
}
