//! The database: connecting a pool to it, bringing its schema up to date, and handing the
//! log and the blobs their connections.

use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::{
    Manager, ManagerConfig, Object, Pool, PoolError, RecyclingMethod, Runtime, TimeoutType,
};
use glave_event::PrincipalId;
use glave_keys::{KeySettings, Keys};
use glave_ports::search::{MOST_CACHED, SearchCache};
use glave_ports::{Clock, Objects, OrgId};
use thiserror::Error;
use tokio_postgres::Config;
use tokio_postgres::error::SqlState;

use crate::blobs::PostgresBlobs;
use crate::directory::PostgresDirectory;
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

/// How long reaching the database's port may take, unless its URL sets `connect_timeout`:
/// moments on any network, so one that has not answered by then is down or out of reach.
const CONNECTS_WITHIN: Duration = Duration::from_secs(10);
/// How long making a connection may take in all: reaching the database, TLS, signing in.
const OPENS_WITHIN: Duration = Duration::from_secs(30);
/// How long a call waits for one of the pool's connections to come free: each is held for one
/// query or transaction, moments, so none free by then means they are stuck behind something.
const FREE_WITHIN: Duration = Duration::from_secs(30);
/// How long one query may run, a wait for a lock included: every query glave makes is moments,
/// one statement of a transaction held for one call, so one still running by then is stuck
/// behind something, and holds up the calls waiting for its connection as long (#1345).
/// Bringing the schema up to date has its own, longer limit (`schema::MIGRATES_WITHIN`).
const QUERIES_WITHIN: Duration = Duration::from_secs(30);
/// How long a transaction may sit idle between its statements: glave sends them back to back,
/// with nothing slow between (no upload, no model), so one idle this long was left by a glave
/// that stopped answering, and holds its locks for nothing.
const IDLE_WITHIN: Duration = Duration::from_secs(30);
/// How long a connection may sit quiet before the system asks whether the database is still
/// there, how often it asks again and how many go unanswered before it gives up: a database
/// that vanished is found in about a minute, not the system's two hours.
const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
const KEEPALIVE_RETRIES: u32 = 3;
/// How long what glave sent may go unacknowledged before the system gives the connection up
/// (Linux): a database that vanishes in the middle of a request is noticed in a minute, where
/// keepalives only ask a quiet connection.
const UNACKNOWLEDGED_WITHIN: Duration = Duration::from_secs(60);

/// One Postgres database: the event log, the blobs' records and their keys, through a bounded
/// connection pool; with the objects that hold the blobs' bytes.
#[derive(Clone)]
pub struct Store {
    pool: Pool,
    keys: Arc<PostgresKeyring>,
    objects: Arc<dyn Objects>,
    /// The keys' clock, which times uploads for the sweep too.
    clock: Arc<dyn Clock>,
    /// The search index's rows, opened, for every log of this store.
    index: Arc<SearchCache>,
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
        mut config: Config,
        keys: KeySettings,
        objects: Arc<dyn Objects>,
    ) -> Result<Self, StoreError> {
        if config.get_connect_timeout().is_none() {
            config.connect_timeout(CONNECTS_WITHIN);
        }
        let chosen = chose_query_limit(&config);
        limited(&mut config);
        let manager = Manager::from_config(
            config,
            crate::tls::connector()?,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .runtime(Runtime::Tokio1)
            .create_timeout(Some(OPENS_WITHIN))
            .wait_timeout(Some(FREE_WITHIN))
            .build()
            .map_err(|e| StoreError::Connect(e.to_string()))?;
        let mut client = pool
            .get()
            .await
            .map_err(|e| StoreError::Connect(unavailable(&e)))?;
        schema::refuse_bypassing(&client).await?;
        queries_limited(&client, chosen).await?;
        schema::migrate(&mut client, &schema::SCHEMA).await?;
        drop(client);
        let clock = Arc::clone(&keys.clock);
        let keys = Arc::new(Keys::new(keys, PostgresKeys::new(pool.clone())));
        let store = Self {
            pool,
            keys,
            objects,
            clock,
            index: Arc::new(SearchCache::new(MOST_CACHED)),
        };
        crate::views::refresh(&store.log(), &store.pool).await?;
        Ok(store)
    }

    pub fn log(&self) -> PostgresLog {
        PostgresLog::new(
            self.pool.clone(),
            Arc::clone(&self.keys),
            Arc::clone(&self.index),
        )
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

    /// The directory sign-in finds people in, and the organizations they belong to.
    pub fn directory(&self) -> PostgresDirectory {
        PostgresDirectory::new(self.pool.clone())
    }

    /// The leases work in flight runs under, on the database's clock.
    /// The organizations the store holds: every one with an organization key, so every one with
    /// a blob. Ids only, for work that runs once per organization, each then in its own scope.
    pub async fn organizations(&self) -> Result<Vec<OrgId>, StoreError> {
        organizations(&self.pool).await
    }

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

    /// The server's own organization, the local person's, made with the id `new` the first time
    /// (`specs/storage.md`, "Many organizations in one store").
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
        let client = unscoped(&self.pool).await?;
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
        let client = unscoped(&self.pool).await?;
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

/// `config` with glave's limits on every connection it opens: the database stops a query or an
/// idle transaction past its limit, and the system finds a database that vanished in about a
/// minute. glave's come before any options the URL gave, so an operator's own setting there
/// wins (the later of two settings holds).
fn limited(config: &mut Config) {
    let limits = format!(
        "-c statement_timeout={} -c idle_in_transaction_session_timeout={}",
        QUERIES_WITHIN.as_millis(),
        IDLE_WITHIN.as_millis()
    );
    let options = match config.get_options() {
        Some(given) if !given.is_empty() => format!("{limits} {given}"),
        _ => limits,
    };
    config
        .options(options)
        .keepalives(true)
        .keepalives_idle(KEEPALIVE_IDLE)
        .keepalives_interval(KEEPALIVE_INTERVAL)
        .keepalives_retries(KEEPALIVE_RETRIES)
        .tcp_user_timeout(UNACKNOWLEDGED_WITHIN);
}

/// Whether the URL's own options set the query limit, which glave then takes as the operator's
/// choice, whatever it is.
fn chose_query_limit(config: &Config) -> bool {
    config
        .get_options()
        .is_some_and(|given| given.contains("statement_timeout"))
}

/// Refuses a database whose connections run queries with no time limit, unless the operator
/// chose that in the URL: the setting glave sends as each connection opens is dropped by a
/// connection pooler between that ignores it, which would leave every query unbounded unseen.
async fn queries_limited(client: &Object, chosen: bool) -> Result<(), StoreError> {
    let limit: String = client
        .query_one("SELECT current_setting('statement_timeout')", &[])
        .await
        .and_then(|row| row.try_get(0))
        .map_err(|e| StoreError::Connect(describe(&e)))?;
    unlimited_refused(&limit, chosen)
}

/// The refusal for a database whose query limit is `limit` (`0` for none), unless chosen.
fn unlimited_refused(limit: &str, chosen: bool) -> Result<(), StoreError> {
    if chosen || limit != "0" {
        return Ok(());
    }
    Err(StoreError::Connect(format!(
        "the database did not take the {} limit glave sets on each query, so a stuck query \
         would never end; a connection pooler between may be dropping it: set \
         statement_timeout for glave's database user, or in the database URL's options, and \
         start again",
        glave_ports::spoken(QUERIES_WITHIN)
    )))
}

/// Borrows a connection from the pool for work in `org`: row-level security then lets it read
/// and write `org`'s rows and no other organization's (`specs/storage.md`, "Row-level
/// security"). Every connection that reads or writes rows is taken here or in [`unscoped`],
/// each setting `glave.org` before anything else, so a pooled connection never carries one
/// organization's setting into another's work.
pub(crate) async fn connection(pool: &Pool, org: OrgId) -> Result<Object, StoreError> {
    borrowed(pool, &org.to_string()).await
}

/// Borrows a connection from the pool for what is no organization's: the schema, the store's
/// own settings and the list of its organizations. It reads and writes no organization's rows.
pub(crate) async fn unscoped(pool: &Pool) -> Result<Object, StoreError> {
    borrowed(pool, "").await
}

async fn borrowed(pool: &Pool, org: &str) -> Result<Object, StoreError> {
    let client = pool
        .get()
        .await
        .map_err(|e| StoreError::Storage(unavailable(&e)))?;
    let failed = |e: tokio_postgres::Error| StoreError::Storage(describe(&e));
    let set = client
        .prepare_cached("SELECT set_config('glave.org', $1, false)")
        .await
        .map_err(failed)?;
    client.execute(&set, &[&org]).await.map_err(failed)?;
    Ok(client)
}

/// The organizations the store holds: every one with an organization key, made the first time
/// anything of it is sealed. Ids only, for work that runs once per organization.
pub(crate) async fn organizations(pool: &Pool) -> Result<Vec<OrgId>, StoreError> {
    let client = unscoped(pool).await?;
    let rows = client
        .query("SELECT org FROM glave_orgs ORDER BY org", &[])
        .await
        .map_err(|e| StoreError::Storage(describe(&e)))?;
    rows.iter()
        .map(|row| {
            row.try_get::<_, Uuid>(0)
                .map(OrgId::from_uuid)
                .map_err(|e| StoreError::Storage(describe(&e)))
        })
        .collect()
}

/// Why the pool gave no connection, a limit that passed saying which and how long it was.
fn unavailable(error: &PoolError) -> String {
    match error {
        PoolError::Timeout(TimeoutType::Wait) => format!(
            "no connection to the database came free within {}",
            glave_ports::spoken(FREE_WITHIN)
        ),
        PoolError::Timeout(TimeoutType::Create) => format!(
            "the database did not answer within {}",
            glave_ports::spoken(OPENS_WITHIN)
        ),
        other => format!("no connection: {other}"),
    }
}

/// What the database said, not just that it failed: the driver's own message for a database
/// error is only "db error". A limit that ran out says which and how long it was.
pub(crate) fn describe(error: &tokio_postgres::Error) -> String {
    match error.as_db_error() {
        Some(said) if timed_out(said) => format!(
            "a query ran past its time limit ({} unless the database URL sets another), and \
             the database stopped it",
            glave_ports::spoken(QUERIES_WITHIN)
        ),
        Some(said) if said.code() == &SqlState::IDLE_IN_TRANSACTION_SESSION_TIMEOUT => format!(
            "a transaction sat idle past its limit of {}, and the database closed its connection",
            glave_ports::spoken(IDLE_WITHIN)
        ),
        Some(said) => said.to_string(),
        None => error.to_string(),
    }
}

/// Whether the database stopped a query at its time limit, not for another reason (an
/// operator's cancel reads the same code).
pub(crate) fn timed_out(said: &tokio_postgres::error::DbError) -> bool {
    said.code() == &SqlState::QUERY_CANCELED && said.message().contains("statement timeout")
}

/// A database failure as a port reports it, through that port's `storage` variant.
pub(crate) fn database<E>(storage: fn(String) -> E) -> impl Fn(tokio_postgres::Error) -> E {
    move |error| storage(format!("the database failed: {}", describe(&error)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_that_takes_the_connection_and_never_answers_is_given_up_saying_so() {
        // Listening, never answering: the system takes the connection and nothing reads it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let url =
            format!("postgres://glave@127.0.0.1:{port}/glave?sslmode=disable&connect_timeout=600");
        let keys = KeySettings::new(Arc::new(glave_keys::InMemoryRoot::new().unwrap()));
        let objects = Arc::new(glave_ports::InMemoryObjects::new());
        let opened = glave_ports::block_on!(async {
            // The clock jumps to the limit as soon as nothing else can run.
            tokio::time::pause();
            Store::connect(&url, keys, objects).await
        });
        let Err(error) = opened else {
            panic!("a database that never answered was opened");
        };
        assert_eq!(
            error.to_string(),
            "could not connect to the database: the database did not answer within 30 seconds"
        );
    }

    #[test]
    fn a_database_that_runs_queries_unlimited_is_refused_unless_the_operator_chose_it() {
        let refused = unlimited_refused("0", false).unwrap_err().to_string();
        assert!(refused.contains("30 seconds limit"), "{refused}");
        assert!(
            unlimited_refused("0", true).is_ok(),
            "the operator's choice"
        );
        assert!(unlimited_refused("30s", false).is_ok());
        assert!(
            unlimited_refused("5min", false).is_ok(),
            "a limit of the role's own"
        );
    }

    #[test]
    fn the_operator_s_own_options_come_after_glave_s_and_win() {
        let mut config: Config = "postgres://glave@db/glave?options=-c%20statement_timeout%3D5s"
            .parse()
            .unwrap();
        assert!(chose_query_limit(&config));
        limited(&mut config);
        assert_eq!(
            config.get_options(),
            Some(
                "-c statement_timeout=30000 -c idle_in_transaction_session_timeout=30000 \
                 -c statement_timeout=5s"
            )
        );
    }
}
