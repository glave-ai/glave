//! The data store (AGENTS.md, "One of each"): the local SQLite file in glave's folder by
//! default, or the Postgres database `[store] url` (or GLAVE_STORE_URL) names, for a server an
//! organization runs. Either way the event log, the blobs, the projects and their keys come from
//! the one store, through the same ports.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use glave_bucket::Bucket;
use glave_config::policy::{Chosen, Source, StoreFile};
use glave_event::{BlobRef, PrincipalId, RawEvent, Seq, Sha256, ThreadId};
use glave_keys::KeyMaterial;
use glave_ports::search::Found;
use glave_ports::{
    Blob, BlobError, BlobStore, Chunks, Eraser, EventLog, Leases, Located, LogError, Objects,
    OrgId, Projects, Scope, Swept, ThreadQuery, ThreadView,
};
use glave_store_postgres as postgres;
use glave_store_sqlite as sqlite;

use crate::system::random_uuid;
use crate::{home, keys};

/// The one data store.
#[derive(Clone)]
pub(crate) enum Data {
    Sqlite(sqlite::Store),
    Postgres(postgres::Store),
}

impl Data {
    /// The store `url` names, or the local SQLite file when it names none; blobs' bytes in the
    /// bucket `bucket` names, or the folder in glave's folder.
    pub(crate) async fn open(url: Option<&str>, bucket: Option<&str>) -> anyhow::Result<Self> {
        // Made first when it is not there yet: the key file may be kept in it.
        let database = home::database().await?;
        let folder = home::folder()?;
        let keys = keys::settings(&folder, home::relocated()).await?;
        let bucket: Arc<dyn Objects> = match bucket {
            Some(bucket) => Arc::new(Bucket::s3(bucket, std::env::vars())?),
            None => Arc::new(Bucket::folder(&home::blobs().await?).await?),
        };
        match url {
            None => Ok(Self::Sqlite(
                sqlite::Store::open(database, keys, bucket).await?,
            )),
            Some(url) => {
                let shown = glave_config::policy::redacted(url);
                let store = postgres::Store::connect(url, keys, bucket)
                    .await
                    .with_context(|| format!("could not open the data store at {shown}"))?;
                Ok(Self::Postgres(store))
            }
        }
    }

    /// The store the choices name: `[store] url` or GLAVE_STORE_URL, else the local file; and
    /// `[store] bucket` or GLAVE_STORE_BUCKET, else the folder.
    pub(crate) async fn chosen(choices: &glave_config::policy::Choices) -> anyhow::Result<Self> {
        if let Some(file) = choices.store.as_ref().and_then(password_in_file) {
            tracing::warn!(
                file = %file.display(),
                "the data store's password is written in this file; it belongs in the \
                 environment, as GLAVE_STORE_URL from the organization's secret manager"
            );
        }
        let url = choices.store.as_ref().map(|url| url.value.as_str());
        let bucket = choices.bucket.as_ref().map(|bucket| bucket.value.as_str());
        Self::open(url, bucket).await
    }

    /// The store for a command that reads no project settings: GLAVE_STORE_URL and
    /// GLAVE_STORE_BUCKET, else the person's own `[store]`, else the local file and folder. The
    /// same rule as [`Data::chosen`], since a project never chooses the store.
    pub(crate) async fn here() -> anyhow::Result<Self> {
        let path = home::folder()?.join(glave_config::policy::CONFIG);
        let file = match tokio::fs::read_to_string(&path).await {
            Ok(text) => glave_config::policy::config(&text, &path)?
                .store
                .unwrap_or_default(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoreFile::default(),
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()));
            }
        };
        let set = environment().unwrap_or_default();
        let url = set.url.or(file.url);
        let bucket = set.bucket.or(file.bucket);
        Self::open(url.as_deref(), bucket.as_deref()).await
    }

    pub(crate) fn log(&self) -> DataLog {
        match self {
            Self::Sqlite(store) => DataLog::Sqlite(store.log()),
            Self::Postgres(store) => DataLog::Postgres(store.log()),
        }
    }

    pub(crate) fn blobs(&self) -> DataBlobs {
        match self {
            Self::Sqlite(store) => DataBlobs::Sqlite(store.blobs()),
            Self::Postgres(store) => DataBlobs::Postgres(store.blobs()),
        }
    }

    pub(crate) fn projects(&self) -> Arc<dyn Projects> {
        match self {
            Self::Sqlite(store) => Arc::new(store.projects()),
            Self::Postgres(store) => Arc::new(store.projects()),
        }
    }

    /// The leases work in flight runs under, on the store's own clock.
    pub(crate) fn leases(&self) -> Arc<dyn Leases> {
        match self {
            Self::Sqlite(store) => Arc::new(store.leases()),
            Self::Postgres(store) => Arc::new(store.leases()),
        }
    }

    /// The keys, for what erases by destroying one.
    pub(crate) fn eraser(&self) -> Arc<dyn Eraser> {
        match self {
            Self::Sqlite(store) => store.keyring(),
            Self::Postgres(store) => store.keyring(),
        }
    }

    /// The one organization this store holds, made the first time.
    pub(crate) async fn org(&self) -> anyhow::Result<OrgId> {
        Ok(match self {
            Self::Sqlite(store) => store.org(random_uuid()?).await?,
            Self::Postgres(store) => store.org(random_uuid()?).await?,
        })
    }

    /// The person's server's id, which browsers check, made with the id `new` the first time.
    pub(crate) async fn server(&self, new: uuid::Uuid) -> anyhow::Result<uuid::Uuid> {
        Ok(match self {
            Self::Sqlite(store) => store.server(new).await?,
            Self::Postgres(store) => store.server(new).await?,
        })
    }

    /// What the root derives for `purpose`: the same while the root lasts, however often this
    /// store starts over.
    pub(crate) async fn derived(&self, purpose: &str) -> anyhow::Result<KeyMaterial> {
        Ok(match self {
            Self::Sqlite(store) => store.keys().derive(purpose).await?,
            Self::Postgres(store) => store.keys().derive(purpose).await?,
        })
    }

    /// The local person and their agent, kept as `new` the first time.
    pub(crate) async fn identity(
        &self,
        new: (PrincipalId, PrincipalId),
    ) -> anyhow::Result<(PrincipalId, PrincipalId)> {
        Ok(match self {
            Self::Sqlite(store) => store.identity(new).await?,
            Self::Postgres(store) => store.identity(new).await?,
        })
    }
}

/// The settings file that holds the store's password, when one does: a password belongs in
/// the environment (`specs/storage.md`, "Which stores"). Only the file is named, never the URL.
fn password_in_file(store: &Chosen<String>) -> Option<&Path> {
    match &store.from {
        Source::File(file) if glave_config::policy::has_password(&store.value) => Some(file),
        _ => None,
    }
}

/// The store GLAVE_STORE_URL and GLAVE_STORE_BUCKET name, when either is set and not empty.
pub(crate) fn environment() -> Option<StoreFile> {
    let set = |name| {
        std::env::var(name)
            .ok()
            .filter(|value: &String| !value.trim().is_empty())
    };
    let store = StoreFile {
        url: set("GLAVE_STORE_URL"),
        bucket: set("GLAVE_STORE_BUCKET"),
    };
    (store != StoreFile::default()).then_some(store)
}

/// The event log of the one store.
#[derive(Clone)]
pub(crate) enum DataLog {
    Sqlite(sqlite::SqliteLog),
    Postgres(postgres::PostgresLog),
}

#[deny(clippy::missing_trait_methods)]
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
pub(crate) enum DataBlobs {
    Sqlite(sqlite::SqliteBlobs),
    Postgres(postgres::PostgresBlobs),
}

#[deny(clippy::missing_trait_methods)]
impl BlobStore for DataBlobs {
    async fn put(
        &self,
        scope: &Scope,
        thread: Option<ThreadId>,
        media_type: &str,
        limit: u64,
        bytes: Box<dyn Chunks>,
    ) -> Result<BlobRef, BlobError> {
        match self {
            Self::Sqlite(blobs) => blobs.put(scope, thread, media_type, limit, bytes).await,
            Self::Postgres(blobs) => blobs.put(scope, thread, media_type, limit, bytes).await,
        }
    }

    async fn get(&self, scope: &Scope, sha256: &Sha256) -> Result<Option<Blob>, BlobError> {
        match self {
            Self::Sqlite(blobs) => blobs.get(scope, sha256).await,
            Self::Postgres(blobs) => blobs.get(scope, sha256).await,
        }
    }

    async fn refer(
        &self,
        scope: &Scope,
        sha256: &Sha256,
        thread: ThreadId,
    ) -> Result<bool, BlobError> {
        match self {
            Self::Sqlite(blobs) => blobs.refer(scope, sha256, thread).await,
            Self::Postgres(blobs) => blobs.refer(scope, sha256, thread).await,
        }
    }

    async fn release(&self, scope: &Scope, thread: ThreadId) -> Result<(), BlobError> {
        match self {
            Self::Sqlite(blobs) => blobs.release(scope, thread).await,
            Self::Postgres(blobs) => blobs.release(scope, thread).await,
        }
    }

    async fn sweep(&self, org: OrgId) -> Result<Swept, BlobError> {
        match self {
            Self::Sqlite(blobs) => blobs.sweep(org).await,
            Self::Postgres(blobs) => blobs.sweep(org).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn only_a_password_written_in_a_file_is_pointed_at_the_environment() {
        let file = PathBuf::from("/srv/glave/config.toml");
        let store = |value: &str, from: Source| Chosen {
            value: value.to_owned(),
            from,
        };
        let written = store(
            "postgres://glave:s3cret@db/glave",
            Source::File(file.clone()),
        );
        assert_eq!(password_in_file(&written), Some(file.as_path()));
        let set = store(
            "postgres://glave:s3cret@db/glave",
            Source::Env("GLAVE_STORE_URL"),
        );
        assert_eq!(password_in_file(&set), None);
        let none = store("postgres://glave@db/glave", Source::File(file));
        assert_eq!(password_in_file(&none), None);
    }
}
