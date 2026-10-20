//! The data store (AGENTS.md, "One of each"): the local SQLite file in glave's folder by
//! default, or the Postgres database `[store] url` (or GLAVE_STORE_URL) names, for a server an
//! organization runs. Either way the event log, the blobs, the projects and their keys come from
//! the one store, through the same ports.

use std::sync::Arc;

use anyhow::Context as _;
use glave_event::{BlobRef, PrincipalId, RawEvent, Seq, Sha256, ThreadId};
use glave_ports::search::Found;
use glave_ports::{
    BlobError, BlobStore, Eraser, EventLog, Located, LogError, OrgId, Projects, Scope, ThreadQuery,
    ThreadView,
};
use glave_store_postgres as postgres;
use glave_store_sqlite as sqlite;

use crate::system::random_uuid;
use crate::{home, keys};

/// The one data store.
#[derive(Clone)]
pub enum Data {
    Sqlite(sqlite::Store),
    Postgres(postgres::Store),
}

impl Data {
    /// The store `url` names, or the local SQLite file when it names none.
    pub async fn open(url: Option<&str>) -> anyhow::Result<Self> {
        // Made first when it is not there yet: the key file may be kept in it.
        let database = home::database().await?;
        let folder = home::folder()?;
        let keys = keys::settings(&folder, home::relocated()).await?;
        match url {
            None => Ok(Self::Sqlite(sqlite::Store::open(database, keys).await?)),
            Some(url) => {
                let shown = glave_config::policy::redacted(url);
                let store = postgres::Store::connect(url, keys)
                    .await
                    .with_context(|| format!("could not open the data store at {shown}"))?;
                Ok(Self::Postgres(store))
            }
        }
    }

    /// The store the choices name: `[store] url` or GLAVE_STORE_URL, else the local file.
    pub async fn chosen(choices: &glave_config::policy::Choices) -> anyhow::Result<Self> {
        Self::open(choices.store.as_ref().map(|url| url.value.as_str())).await
    }

    /// The store for a command that reads no project settings: GLAVE_STORE_URL, else the
    /// person's own `[store] url`, else the local file. The same rule as [`Data::chosen`],
    /// since a project never chooses the store.
    pub async fn here() -> anyhow::Result<Self> {
        if let Some(url) = environment() {
            return Self::open(Some(&url)).await;
        }
        let path = home::folder()?.join(glave_config::policy::CONFIG);
        let url = match tokio::fs::read_to_string(&path).await {
            Ok(text) => glave_config::policy::config(&text, &path)?
                .store
                .map(|store| store.url),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()));
            }
        };
        Self::open(url.as_deref()).await
    }

    pub fn log(&self) -> DataLog {
        match self {
            Self::Sqlite(store) => DataLog::Sqlite(store.log()),
            Self::Postgres(store) => DataLog::Postgres(store.log()),
        }
    }

    pub fn blobs(&self) -> DataBlobs {
        match self {
            Self::Sqlite(store) => DataBlobs::Sqlite(store.blobs()),
            Self::Postgres(store) => DataBlobs::Postgres(store.blobs()),
        }
    }

    pub fn projects(&self) -> Arc<dyn Projects> {
        match self {
            Self::Sqlite(store) => Arc::new(store.projects()),
            Self::Postgres(store) => Arc::new(store.projects()),
        }
    }

    /// The keys, for what erases by destroying one.
    pub fn eraser(&self) -> Arc<dyn Eraser> {
        match self {
            Self::Sqlite(store) => store.keyring(),
            Self::Postgres(store) => store.keyring(),
        }
    }

    /// The one organization this store holds, made the first time.
    pub async fn org(&self) -> anyhow::Result<OrgId> {
        Ok(match self {
            Self::Sqlite(store) => store.org(random_uuid()?).await?,
            Self::Postgres(store) => store.org(random_uuid()?).await?,
        })
    }

    /// The person's server's id, which browsers check, made with the id `new` the first time.
    pub async fn server(&self, new: uuid::Uuid) -> anyhow::Result<uuid::Uuid> {
        Ok(match self {
            Self::Sqlite(store) => store.server(new).await?,
            Self::Postgres(store) => store.server(new).await?,
        })
    }

    /// The local person and their agent, kept as `new` the first time.
    pub async fn identity(
        &self,
        new: (PrincipalId, PrincipalId),
    ) -> anyhow::Result<(PrincipalId, PrincipalId)> {
        Ok(match self {
            Self::Sqlite(store) => store.identity(new).await?,
            Self::Postgres(store) => store.identity(new).await?,
        })
    }
}

/// The store GLAVE_STORE_URL names, when it is set and not empty.
pub fn environment() -> Option<String> {
    std::env::var("GLAVE_STORE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
}

/// The event log of the one store.
#[derive(Clone)]
pub enum DataLog {
    Sqlite(sqlite::SqliteLog),
    Postgres(postgres::PostgresLog),
}

impl EventLog for DataLog {
    async fn append(&self, scope: &Scope, events: &[RawEvent]) -> Result<(), LogError> {
        match self {
            Self::Sqlite(log) => log.append(scope, events).await,
            Self::Postgres(log) => log.append(scope, events).await,
        }
    }

    async fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> Result<Vec<RawEvent>, LogError> {
        match self {
            Self::Sqlite(log) => log.read(scope, thread, after, limit).await,
            Self::Postgres(log) => log.read(scope, thread, after, limit).await,
        }
    }

    async fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<Found>, LogError> {
        match self {
            Self::Sqlite(log) => log.search(scope, query, limit).await,
            Self::Postgres(log) => log.search(scope, query, limit).await,
        }
    }

    async fn locate(&self, thread: ThreadId) -> Result<Option<Located>, LogError> {
        match self {
            Self::Sqlite(log) => log.locate(thread).await,
            Self::Postgres(log) => log.locate(thread).await,
        }
    }

    async fn thread_views(
        &self,
        scope: &Scope,
        query: &ThreadQuery,
    ) -> Result<Vec<ThreadView>, LogError> {
        match self {
            Self::Sqlite(log) => log.thread_views(scope, query).await,
            Self::Postgres(log) => log.thread_views(scope, query).await,
        }
    }

    async fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<ThreadView>, LogError> {
        match self {
            Self::Sqlite(log) => log.thread_view(scope, thread).await,
            Self::Postgres(log) => log.thread_view(scope, thread).await,
        }
    }

    async fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<ThreadView>, LogError> {
        match self {
            Self::Sqlite(log) => log.rebuild_thread_view(scope, thread).await,
            Self::Postgres(log) => log.rebuild_thread_view(scope, thread).await,
        }
    }

    async fn latest_of(
        &self,
        scope: &Scope,
        thread: ThreadId,
        kinds: &[&str],
    ) -> Result<Option<RawEvent>, LogError> {
        match self {
            Self::Sqlite(log) => log.latest_of(scope, thread, kinds).await,
            Self::Postgres(log) => log.latest_of(scope, thread, kinds).await,
        }
    }
}

/// The blob store of the one store.
#[derive(Clone)]
pub enum DataBlobs {
    Sqlite(sqlite::SqliteBlobs),
    Postgres(postgres::PostgresBlobs),
}

impl BlobStore for DataBlobs {
    async fn put(
        &self,
        scope: &Scope,
        thread: ThreadId,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> Result<BlobRef, BlobError> {
        match self {
            Self::Sqlite(blobs) => blobs.put(scope, thread, bytes, media_type).await,
            Self::Postgres(blobs) => blobs.put(scope, thread, bytes, media_type).await,
        }
    }

    async fn get(&self, scope: &Scope, sha256: &Sha256) -> Result<Option<Vec<u8>>, BlobError> {
        match self {
            Self::Sqlite(blobs) => blobs.get(scope, sha256).await,
            Self::Postgres(blobs) => blobs.get(scope, sha256).await,
        }
    }
}
