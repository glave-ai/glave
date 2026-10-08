//! The database file: opening and migrating it, the local organization and the space for each
//! project folder, and running work off the async runtime.

use std::path::Path;
use std::sync::{Arc, Mutex};

use glave_ports::{OrgId, Scope, SpaceId};
use rusqlite::{Connection, OptionalExtension as _, params};
use thiserror::Error;
use uuid::Uuid;

use crate::blobs::SqliteBlobs;
use crate::log::SqliteLog;
use crate::schema;

/// Why the store could not open or answer.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("could not open the database: {0}")]
    Open(String),
    #[error("the database schema is not usable: {0}")]
    Schema(String),
    #[error("the database failed: {0}")]
    Storage(String),
}

impl StoreError {
    pub(crate) fn schema(error: &rusqlite::Error) -> Self {
        Self::Schema(error.to_string())
    }
}

pub(crate) type Shared = Arc<Mutex<Connection>>;

/// One local database: the event log, the blobs, and which space each folder is.
#[derive(Clone)]
pub struct Store {
    connection: Shared,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and brings its schema up to date.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_owned();
        tokio::task::spawn_blocking(move || {
            let connection = Connection::open(&path)
                .map_err(|e| StoreError::Open(format!("{}: {e}", path.display())))?;
            Self::prepare(connection)
        })
        .await
        .map_err(|e| StoreError::Open(e.to_string()))?
    }

    /// A database that lives only in memory, for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let connection =
            Connection::open_in_memory().map_err(|e| StoreError::Open(e.to_string()))?;
        Self::prepare(connection)
    }

    fn prepare(mut connection: Connection) -> Result<Self, StoreError> {
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;",
            )
            .map_err(|e| StoreError::Open(e.to_string()))?;
        schema::migrate(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn log(&self) -> SqliteLog {
        SqliteLog::new(Arc::clone(&self.connection))
    }

    pub fn blobs(&self) -> SqliteBlobs {
        SqliteBlobs::new(Arc::clone(&self.connection))
    }

    /// The scope for a project folder on this computer: the one local organization, and the
    /// space this folder was given the first time. `new_org` and `new_space` are used only when
    /// those do not exist yet.
    pub async fn local_scope(
        &self,
        folder: &Path,
        new_org: Uuid,
        new_space: Uuid,
    ) -> Result<Scope, StoreError> {
        let folder = folder.to_string_lossy().into_owned();
        run(&self.connection, move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT OR IGNORE INTO settings (key, value) VALUES ('org', ?1)",
                params![new_org.to_string()],
            )?;
            let org: String = transaction.query_row(
                "SELECT value FROM settings WHERE key = 'org'",
                [],
                |row| row.get(0),
            )?;
            transaction.execute(
                "INSERT OR IGNORE INTO spaces (path, id) VALUES (?1, ?2)",
                params![folder, new_space.to_string()],
            )?;
            let space: Option<String> = transaction
                .query_row(
                    "SELECT id FROM spaces WHERE path = ?1",
                    params![folder],
                    |row| row.get(0),
                )
                .optional()?;
            transaction.commit()?;
            Ok((org, space))
        })
        .await
        .and_then(|(org, space)| {
            let parse = |text: &str| {
                Uuid::parse_str(text)
                    .map_err(|e| StoreError::Storage(format!("bad id {text:?}: {e}")))
            };
            let space = space
                .ok_or_else(|| StoreError::Storage("the folder's space was not recorded".into()))?;
            Ok(Scope {
                org: OrgId::from_uuid(parse(&org)?),
                space: SpaceId::from_uuid(parse(&space)?),
            })
        })
    }
}

/// Runs database work on the blocking pool, so the async runtime never waits on disk.
pub(crate) async fn run<T, F>(connection: &Shared, work: F) -> Result<T, StoreError>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
{
    let connection = Arc::clone(connection);
    tokio::task::spawn_blocking(move || {
        let mut guard = connection
            .lock()
            .map_err(|_| StoreError::Storage("the database lock is poisoned".into()))?;
        work(&mut guard).map_err(|e| StoreError::Storage(e.to_string()))
    })
    .await
    .map_err(|e| StoreError::Storage(e.to_string()))?
}
