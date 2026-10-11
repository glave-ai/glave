//! The database: connecting a pool to it, bringing its schema up to date, and handing the
//! log and the blobs their connections.

use std::sync::Arc;

use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod};
use glave_keys::{KeySettings, Keys};
use thiserror::Error;
use tokio_postgres::{Config, NoTls};

use crate::blobs::PostgresBlobs;
use crate::keys::PostgresKeys;
use crate::log::PostgresLog;
use crate::schema;
use crate::spaces::PostgresSpaces;

/// Why the store could not open or answer.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("could not connect to the database: {0}")]
    Connect(String),
    #[error("the database schema is not usable: {0}")]
    Schema(String),
    #[error("the database failed: {0}")]
    Storage(String),
}

/// The keys of the database.
pub type PostgresKeyring = Keys<PostgresKeys>;

/// One Postgres database: the event log, the blobs and their keys, through a bounded
/// connection pool.
#[derive(Clone)]
pub struct Store {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
}

impl Store {
    /// Connects to the database at `url` (`postgres://user:password@host/database`) and
    /// brings its schema up to date. Content is sealed under keys that `keys.root` wraps.
    pub async fn connect(url: &str, keys: KeySettings) -> Result<Self, StoreError> {
        let config = url
            .parse::<Config>()
            .map_err(|e| StoreError::Connect(format!("the database URL is not valid: {e}")))?;
        Self::connect_with(config, keys).await
    }

    /// Connects with a prepared configuration, for settings a URL cannot carry, and brings the
    /// schema up to date. The tables live in the connection's `search_path`.
    pub async fn connect_with(config: Config, keys: KeySettings) -> Result<Self, StoreError> {
        let manager = Manager::from_config(
            config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .build()
            .map_err(|e| StoreError::Connect(e.to_string()))?;
        let mut client = pool
            .get()
            .await
            .map_err(|e| StoreError::Connect(e.to_string()))?;
        schema::migrate(&mut client).await?;
        let keys = Arc::new(Keys::new(keys, PostgresKeys::new(pool.clone())));
        Ok(Self { pool, keys })
    }

    pub fn log(&self) -> PostgresLog {
        PostgresLog::new(self.pool.clone(), Arc::clone(&self.keys))
    }

    pub fn blobs(&self) -> PostgresBlobs {
        PostgresBlobs::new(self.pool.clone(), Arc::clone(&self.keys))
    }

    /// The spaces: their records, members, and the folders that lead to them.
    pub fn spaces(&self) -> PostgresSpaces {
        PostgresSpaces::new(self.pool.clone())
    }

    /// The keys content is sealed under, and erased by destroying.
    pub fn keys(&self) -> &PostgresKeyring {
        &self.keys
    }
}

/// Borrows a connection from the pool.
pub(crate) async fn connection(pool: &Pool) -> Result<Object, StoreError> {
    pool.get()
        .await
        .map_err(|e| StoreError::Storage(format!("no connection: {e}")))
}

/// What the database said, not just that it failed: the driver's own message for a database
/// error is only "db error".
pub(crate) fn describe(error: &tokio_postgres::Error) -> String {
    error
        .as_db_error()
        .map_or_else(|| error.to_string(), ToString::to_string)
}
