//! The database: connecting a pool to it, bringing its schema up to date, and handing the
//! log and the blobs their connections.

use std::sync::Arc;

use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod};
use glave_event::PrincipalId;
use glave_keys::{KeySettings, Keys};
use glave_ports::{Clock, Objects, OrgId};
use thiserror::Error;
use tokio_postgres::Config;

use crate::blobs::PostgresBlobs;
use crate::keys::PostgresKeys;
use crate::leases::PostgresLeases;
use crate::log::PostgresLog;
use crate::projects::PostgresProjects;
use crate::schema;
use uuid::Uuid;

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

/// One Postgres database: the event log, the blobs' records and their keys, through a bounded
/// connection pool; with the objects that hold the blobs' bytes.
#[derive(Clone)]
pub struct Store {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
    objects: Arc<dyn Objects>,
    /// The keys' clock, which times uploads for the sweep too.
    clock: Arc<dyn Clock>,
}

impl Store {
    /// Connects to the database at `url` (`postgres://user:password@host/database`) and
    /// brings its schema up to date. Content is sealed under keys that `keys.root` wraps, and
    /// blobs' bytes kept in `objects`. A database on another computer is reached over TLS
    /// unless the URL says `sslmode=disable`.
    pub async fn connect(
        url: &str,
        keys: KeySettings,
        objects: Arc<dyn Objects>,
    ) -> Result<Self, StoreError> {
        // The URL may hold a password, so the error never repeats it.
        let mut config = url
            .parse::<Config>()
            .map_err(|e| StoreError::Connect(format!("the database URL is not valid: {e}")))?;
        crate::tls::require_off_this_computer(&mut config);
        Self::connect_with(config, keys, objects).await
    }

    /// Connects with a prepared configuration, for settings a URL cannot carry, and brings the
    /// schema up to date. The tables live in the connection's `search_path`. The threads view
    /// is filled from the log the first time, and again whenever this build folds threads
    /// differently from the one that filled it.
    pub async fn connect_with(
        config: Config,
        keys: KeySettings,
        objects: Arc<dyn Objects>,
    ) -> Result<Self, StoreError> {
        let manager = Manager::from_config(
            config,
            crate::tls::connector()?,
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
        schema::prepare(&mut client).await?;
        drop(client);
        let clock = Arc::clone(&keys.clock);
        let keys = Arc::new(Keys::new(keys, PostgresKeys::new(pool.clone())));
        let store = Self {
            pool,
            keys,
            objects,
            clock,
        };
        crate::views::refresh(&store.log(), &store.pool).await?;
        Ok(store)
    }

    pub fn log(&self) -> PostgresLog {
        PostgresLog::new(self.pool.clone(), Arc::clone(&self.keys))
    }

    pub fn blobs(&self) -> PostgresBlobs {
        PostgresBlobs::new(
            self.pool.clone(),
            Arc::clone(&self.keys),
            Arc::clone(&self.objects),
            Arc::clone(&self.clock),
        )
    }

    /// The projects: their records, and the folders that lead to them.
    pub fn projects(&self) -> PostgresProjects {
        PostgresProjects::new(self.pool.clone())
    }

    /// The leases work in flight runs under, on the database's clock.
    pub fn leases(&self) -> PostgresLeases {
        PostgresLeases::new(self.pool.clone())
    }

    /// The keys content is sealed under, and erased by destroying.
    pub fn keys(&self) -> &PostgresKeyring {
        &self.keys
    }

    /// The keys, shared, for what destroys them (memory forgets by erasing a key).
    pub fn keyring(&self) -> Arc<PostgresKeyring> {
        Arc::clone(&self.keys)
    }

    /// The organization this database holds, made with the id `new` the first time.
    pub async fn org(&self, new: Uuid) -> Result<OrgId, StoreError> {
        self.kept_once("org", new).await.map(OrgId::from_uuid)
    }

    /// The person's server's id, which browsers check before they present a session, made
    /// with the id `new` the first time: every server on this database is the same one to a
    /// browser.
    pub async fn server(&self, new: Uuid) -> Result<Uuid, StoreError> {
        self.kept_once("server", new).await
    }

    /// The id kept under `key`, kept as `new` the first time.
    async fn kept_once(&self, key: &str, new: Uuid) -> Result<Uuid, StoreError> {
        let client = connection(&self.pool).await?;
        let failed = |e: tokio_postgres::Error| StoreError::Storage(describe(&e));
        client
            .execute(
                "INSERT INTO glave_settings (key, value) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                &[&key, &new.to_string()],
            )
            .await
            .map_err(failed)?;
        let kept: String = client
            .query_one("SELECT value FROM glave_settings WHERE key = $1", &[&key])
            .await
            .map_err(failed)?
            .try_get(0)
            .map_err(failed)?;
        Uuid::parse_str(&kept)
            .map_err(|e| StoreError::Storage(format!("bad {key} id {kept:?}: {e}")))
    }

    /// The local person and their agent, `(person, agent)`, kept as `new` the first time, both
    /// in one statement, so no crash keeps one without the other, and every server on this
    /// database sees the same two.
    pub async fn identity(
        &self,
        new: (PrincipalId, PrincipalId),
    ) -> Result<(PrincipalId, PrincipalId), StoreError> {
        let client = connection(&self.pool).await?;
        let failed = |e: tokio_postgres::Error| StoreError::Storage(describe(&e));
        client
            .execute(
                "INSERT INTO glave_settings (key, value) VALUES ('person', $1), ('agent', $2)
                 ON CONFLICT DO NOTHING",
                &[&new.0.to_string(), &new.1.to_string()],
            )
            .await
            .map_err(failed)?;
        let rows = client
            .query(
                "SELECT value FROM glave_settings WHERE key IN ('person', 'agent')
                 ORDER BY key DESC",
                &[],
            )
            .await
            .map_err(failed)?;
        let kept = rows
            .iter()
            .map(|row| row.try_get::<_, String>(0))
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        match kept.as_slice() {
            [person, agent] => Ok((principal(person)?, principal(agent)?)),
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

/// A database failure as a port reports it, through that port's `storage` variant.
pub(crate) fn database<E>(storage: fn(String) -> E) -> impl Fn(tokio_postgres::Error) -> E {
    move |error| storage(format!("the database failed: {}", describe(&error)))
}
