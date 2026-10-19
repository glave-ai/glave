//! The blob store on SQLite: content kept once per scope, sealed, and stored under an HMAC of
//! its SHA-256 so the file does not reveal which known content it holds.

use std::future::Future;
use std::sync::Arc;

use glave_event::{BlobRef, Sha256, ThreadId};
use glave_ports::{BlobError, BlobStore, Scope, digest};
use rusqlite::{OptionalExtension as _, params};

use crate::store::{Shared, SqliteKeyring, run};

/// The blob store in the local database.
#[derive(Clone)]
pub struct SqliteBlobs {
    connection: Shared,
    keys: Arc<SqliteKeyring>,
}

impl SqliteBlobs {
    pub(crate) fn new(connection: Shared, keys: Arc<SqliteKeyring>) -> Self {
        Self { connection, keys }
    }
}

impl BlobStore for SqliteBlobs {
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
            let reference = BlobRef {
                sha256: sha256.clone(),
                bytes: size,
                media_type: media_type.clone(),
            };
            let (name, sealed) = this
                .keys
                .seal_blob(&scope, &sha256, &bytes)
                .await
                .map_err(|e| BlobError::Storage(format!("could not seal a blob: {e}")))?;
            run(&this.connection, move |connection| {
                let (org, project) = (scope.org.to_string(), scope.project.to_string());
                let transaction = connection.transaction()?;
                // Storing it again re-seals it under this month's key.
                transaction.execute(
                    "INSERT INTO blobs (org, project, name, media_type, bytes, key_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (org, project, name) DO UPDATE
                     SET media_type = excluded.media_type, bytes = excluded.bytes,
                         key_id = excluded.key_id",
                    params![org, project, name, media_type, sealed.bytes, sealed.key_id],
                )?;
                transaction.execute(
                    "INSERT OR IGNORE INTO blob_threads (org, project, name, thread)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![org, project, name, thread.to_string()],
                )?;
                transaction.commit()
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
        let this = self.clone();
        async move {
            let name = this
                .keys
                .blob_name(&scope, &sha256)
                .await
                .map_err(|e| BlobError::Storage(e.to_string()))?;
            // A project with no key yet (or an erased one) has nothing that can be found.
            let Some(name) = name else {
                return Ok(None);
            };
            let lookup = name.clone();
            let row: Option<(Vec<u8>, String)> = run(&this.connection, move |connection| {
                connection
                    .query_row(
                        "SELECT bytes, key_id FROM blobs
                         WHERE org = ?1 AND project = ?2 AND name = ?3",
                        params![scope.org.to_string(), scope.project.to_string(), lookup],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
            })
            .await
            .map_err(|e| BlobError::Storage(e.to_string()))?;
            let Some((bytes, key_id)) = row else {
                return Ok(None);
            };
            this.keys
                .open_blob(&scope, &name, &key_id, &bytes)
                .await
                .map(Some)
                .map_err(|e| BlobError::Storage(format!("a blob could not be opened: {e}")))
        }
    }
}
