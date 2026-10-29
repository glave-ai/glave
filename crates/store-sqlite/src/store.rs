//! The database file: opening or making it, the local organization, and running work off
//! the async runtime.

use std::path::Path;
use std::sync::{Arc, Mutex};

use glave_event::PrincipalId;
use glave_keys::{KeySettings, Keys};
use glave_ports::{Clock, InMemoryObjects, Objects, OrgId};
use rusqlite::{Connection, params};
use thiserror::Error;
use uuid::Uuid;

use crate::blobs::SqliteBlobs;
use crate::directory::SqliteDirectory;
use crate::keys::SqliteKeys;
use crate::leases::SqliteLeases;
use crate::log::SqliteLog;
use crate::projects::SqliteProjects;
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

/// The keys of the local database.
pub type SqliteKeyring = Keys<SqliteKeys>;

/// One local database: the event log, the blobs' records, their keys, and which project each
/// folder is; with the objects that hold the blobs' bytes.
#[derive(Clone)]
pub struct Store {
    connection: Shared,
    keys: Arc<SqliteKeyring>,
    objects: Arc<dyn Objects>,
    /// The keys' clock, which times uploads for the sweep too.
    clock: Arc<dyn Clock>,
}

impl Store {
    /// Opens the database at `path`, making it the first time; one another version of glave
    /// wrote is refused.
    /// Content is sealed under keys that `keys.root` wraps; blobs' bytes are kept in `objects`.
    /// The threads view is filled from the log the first time, and again whenever this build
    /// folds threads differently from the one that filled it.
    pub async fn open(
        path: impl AsRef<Path>,
        keys: KeySettings,
        objects: Arc<dyn Objects>,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref().to_owned();
        let store = tokio::task::spawn_blocking(move || {
            let connection = Connection::open(&path)
                .map_err(|e| StoreError::Open(format!("{}: {e}", path.display())))?;
            Self::prepare(connection, keys, objects)
        })
        .await
        .map_err(|e| StoreError::Open(e.to_string()))??;
        crate::views::refresh(&store.log(), &store.connection).await?;
        Ok(store)
    }

    /// A database that lives only in memory, its blobs' bytes too, for tests.
    pub fn open_in_memory(keys: KeySettings) -> Result<Self, StoreError> {
        let connection =
            Connection::open_in_memory().map_err(|e| StoreError::Open(e.to_string()))?;
        let objects = InMemoryObjects::with_clock(Arc::clone(&keys.clock));
        Self::prepare(connection, keys, Arc::new(objects))
    }

    fn prepare(
        mut connection: Connection,
        keys: KeySettings,
        objects: Arc<dyn Objects>,
    ) -> Result<Self, StoreError> {
        // `secure_delete` overwrites deleted and replaced content, so a destroyed key does not
        // linger in the file's free pages.
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA secure_delete = ON;",
            )
            .map_err(|e| StoreError::Open(e.to_string()))?;
        schema::prepare(&mut connection)?;
        let connection = Arc::new(Mutex::new(connection));
        let clock = Arc::clone(&keys.clock);
        let keys = Arc::new(Keys::new(keys, SqliteKeys::new(Arc::clone(&connection))));
        Ok(Self {
            connection,
            keys,
            objects,
            clock,
        })
    }

    pub fn log(&self) -> SqliteLog {
        SqliteLog::new(Arc::clone(&self.connection), Arc::clone(&self.keys))
    }

    pub fn blobs(&self) -> SqliteBlobs {
        SqliteBlobs::new(
            Arc::clone(&self.connection),
            Arc::clone(&self.keys),
            Arc::clone(&self.objects),
            Arc::clone(&self.clock),
        )
    }

    /// The keys content is sealed under, and erased by destroying.
    pub fn keys(&self) -> &SqliteKeyring {
        &self.keys
    }

    /// The keys, shared, for what erases by destroying them (forgotten memories).
    pub fn keyring(&self) -> Arc<SqliteKeyring> {
        Arc::clone(&self.keys)
    }

    /// The projects: their records, and the folders that lead to them.
    pub fn projects(&self) -> SqliteProjects {
        SqliteProjects::new(Arc::clone(&self.connection))
    }

    /// The leases work in flight runs under, on this file's clock.
    pub fn leases(&self) -> SqliteLeases {
        SqliteLeases::new(Arc::clone(&self.connection))
    }

    /// The directory sign-in finds people in, and the organizations they belong to.
    pub fn directory(&self) -> SqliteDirectory {
        SqliteDirectory::new(Arc::clone(&self.connection))
    }

    /// The one organization on this computer, made with the id `new` the first time.
    pub async fn org(&self, new: Uuid) -> Result<OrgId, StoreError> {
        self.kept_once("org", new).await.map(OrgId::from_uuid)
    }

    /// The person's server's id, which browsers check before they present a session, made
    /// with the id `new` the first time.
    pub async fn server(&self, new: Uuid) -> Result<Uuid, StoreError> {
        self.kept_once("server", new).await
    }

    /// The id kept under `key`, kept as `new` the first time.
    async fn kept_once(&self, key: &'static str, new: Uuid) -> Result<Uuid, StoreError> {
        let kept: String = run(&self.connection, move |connection| {
            connection.execute(
                "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
                params![key, new.to_string()],
            )?;
            connection.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
        })
        .await?;
        Uuid::parse_str(&kept)
            .map_err(|e| StoreError::Storage(format!("bad {key} id {kept:?}: {e}")))
    }

    /// The local person and their agent, `(person, agent)`, kept as `new` the first time, both
    /// in one statement, so no crash keeps one without the other.
    pub async fn identity(
        &self,
        new: (PrincipalId, PrincipalId),
    ) -> Result<(PrincipalId, PrincipalId), StoreError> {
        let kept: Vec<(String, String)> = run(&self.connection, move |connection| {
            connection.execute(
                "INSERT OR IGNORE INTO settings (key, value) VALUES ('person', ?1), ('agent', ?2)",
                params![new.0.to_string(), new.1.to_string()],
            )?;
            let mut select = connection.prepare(
                "SELECT key, value FROM settings WHERE key IN ('person', 'agent') ORDER BY key DESC",
            )?;
            let rows = select.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect()
        })
        .await?;
        match kept.as_slice() {
            [(_, person), (_, agent)] => Ok((principal(person)?, principal(agent)?)),
            _ => Err(StoreError::Storage(
                "the local person and their agent are not both kept".into(),
            )),
        }
    }
}

/// The principal id kept as `text`.
fn principal(text: &str) -> Result<PrincipalId, StoreError> {
    Uuid::parse_str(text)
        .ok()
        .and_then(|uuid| PrincipalId::try_from(uuid).ok())
        .ok_or_else(|| StoreError::Storage(format!("bad principal id {text:?}")))
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
