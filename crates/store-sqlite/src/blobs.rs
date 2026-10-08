//! The blob store on SQLite: content kept once per scope, named by its SHA-256.

use std::future::Future;

use glave_event::{BlobRef, Sha256};
use glave_ports::{BlobError, BlobStore, Scope, digest};
use rusqlite::{OptionalExtension as _, params};

use crate::store::{Shared, run};

/// The blob store in the local database.
#[derive(Clone)]
pub struct SqliteBlobs {
    connection: Shared,
}

impl SqliteBlobs {
    pub(crate) fn new(connection: Shared) -> Self {
        Self { connection }
    }
}

impl BlobStore for SqliteBlobs {
    fn put(
        &self,
        scope: &Scope,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> impl Future<Output = Result<BlobRef, BlobError>> + Send {
        let scope = *scope;
        let media_type = media_type.to_owned();
        let connection = self.connection.clone();
        async move {
            let sha256 = digest(&bytes);
            let size = u64::try_from(bytes.len()).map_err(|e| BlobError::Storage(e.to_string()))?;
            let reference = BlobRef {
                sha256: sha256.clone(),
                bytes: size,
                media_type: media_type.clone(),
            };
            run(&connection, move |connection| {
                connection.execute(
                    "INSERT OR IGNORE INTO blobs (org, space, sha256, media_type, bytes)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        scope.org.to_string(),
                        scope.space.to_string(),
                        sha256.as_str(),
                        media_type,
                        bytes
                    ],
                )
            })
            .await
            .map_err(|e| BlobError::Storage(e.to_string()))?;
            Ok(reference)
        }
    }

    fn get(
        &self,
        scope: &Scope,
        sha256: &Sha256,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, BlobError>> + Send {
        let scope = *scope;
        let sha256 = sha256.clone();
        let connection = self.connection.clone();
        async move {
            run(&connection, move |connection| {
                connection
                    .query_row(
                        "SELECT bytes FROM blobs WHERE org = ?1 AND space = ?2 AND sha256 = ?3",
                        params![
                            scope.org.to_string(),
                            scope.space.to_string(),
                            sha256.as_str()
                        ],
                        |row| row.get(0),
                    )
                    .optional()
            })
            .await
            .map_err(|e| BlobError::Storage(e.to_string()))
        }
    }
}
