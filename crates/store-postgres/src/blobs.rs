//! The blob store on Postgres: content kept once per scope, named by its SHA-256.

use std::future::Future;

use deadpool_postgres::Pool;
use glave_event::{BlobRef, Sha256};
use glave_ports::{BlobError, BlobStore, Scope, digest};

use crate::store::{connection, describe};

/// The blob store in the database.
#[derive(Clone)]
pub struct PostgresBlobs {
    pool: Pool,
}

impl PostgresBlobs {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

impl BlobStore for PostgresBlobs {
    fn put(
        &self,
        scope: &Scope,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send {
        let scope = *scope;
        let media_type = media_type.to_owned();
        let pool = self.pool.clone();
        async move {
            let sha256 = digest(&bytes);
            let size = u64::try_from(bytes.len()).map_err(|e| BlobError::Storage(e.to_string()))?;
            let client = connection(&pool)
                .await
                .map_err(|e| BlobError::Storage(e.to_string()))?;
            let statement = client
                .prepare_cached(
                    "INSERT INTO blobs (org, space, sha256, media_type, bytes)
                     VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
                )
                .await
                .map_err(database)?;
            client
                .execute(
                    &statement,
                    &[
                        &scope.org.as_uuid(),
                        &scope.space.as_uuid(),
                        &sha256.as_str(),
                        &media_type,
                        &bytes,
                    ],
                )
                .await
                .map_err(database)?;
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
        let pool = self.pool.clone();
        async move {
            let client = connection(&pool)
                .await
                .map_err(|e| BlobError::Storage(e.to_string()))?;
            let statement = client
                .prepare_cached(
                    "SELECT bytes FROM blobs WHERE org = $1 AND space = $2 AND sha256 = $3",
                )
                .await
                .map_err(database)?;
            let row = client
                .query_opt(
                    &statement,
                    &[
                        &scope.org.as_uuid(),
                        &scope.space.as_uuid(),
                        &sha256.as_str(),
                    ],
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
