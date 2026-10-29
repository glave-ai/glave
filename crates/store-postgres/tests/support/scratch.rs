//! A schema of a test's own in the database `GLAVE_TEST_POSTGRES_URL` names, dropped at the
//! end, so tests run in parallel and leave nothing behind. Stores sign in as that URL's role,
//! which row-level security binds; the test looks at every organization's rows through
//! `GLAVE_TEST_POSTGRES_ADMIN_URL`, a superuser's sign-in to the same database.

use std::sync::Arc;

use glave_keys::{InMemoryRoot, KeySettings};
use glave_ports::InMemoryObjects;
pub(crate) use glave_ports::block_on;
use glave_ports::testing::ManualClock;
use glave_store_postgres::{Config, Store, StoreError};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, NoTls};

/// A schema of the test's own, an admin connection to make, inspect and drop it, and the
/// root every store on it shares.
pub(crate) struct Scratch {
    pub admin: Client,
    connection: JoinHandle<()>,
    pub schema: String,
    pub config: Config,
    root: Arc<InMemoryRoot>,
    /// The objects every store on this schema keeps blobs' bytes in.
    pub objects: InMemoryObjects,
    /// The clock every store on this schema, and its objects, read.
    pub clock: ManualClock,
}

impl Scratch {
    /// A fresh, empty schema, or `None` when no database was given.
    pub(crate) async fn new(name: &str) -> Option<Self> {
        let (Ok(url), Ok(admin_url)) = (
            std::env::var("GLAVE_TEST_POSTGRES_URL"),
            std::env::var("GLAVE_TEST_POSTGRES_ADMIN_URL"),
        ) else {
            eprintln!(
                "skipped: set GLAVE_TEST_POSTGRES_URL and GLAVE_TEST_POSTGRES_ADMIN_URL to run \
                 the Postgres tests"
            );
            return None;
        };
        let (admin, connection) = tokio_postgres::connect(&admin_url, NoTls).await.unwrap();
        let connection = tokio::spawn(async move {
            let _ = connection.await;
        });
        // Test processes run one test each, so the process id keeps parallel runs apart.
        let schema = format!("glave_test_{name}_{}", std::process::id());
        let mut config: Config = url.parse().unwrap();
        let role = config.get_user().unwrap().to_owned();
        admin
            .batch_execute(&format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE;
                 CREATE SCHEMA {schema} AUTHORIZATION \"{role}\";"
            ))
            .await
            .unwrap();
        let clock = ManualClock::at_millis(1_791_000_000_000);
        config.options(format!("-c search_path={schema}"));
        Some(Self {
            admin,
            connection,
            schema,
            config,
            root: Arc::new(InMemoryRoot::new().unwrap()),
            objects: InMemoryObjects::with_clock(Arc::new(clock.clone())),
            clock,
        })
    }

    /// Keys under the root this scratch schema's stores share.
    pub(crate) fn keys(&self) -> KeySettings {
        let mut keys = KeySettings::new(Arc::<InMemoryRoot>::clone(&self.root));
        keys.clock = Arc::new(self.clock.clone());
        keys
    }

    pub(crate) async fn store(&self) -> Result<Store, StoreError> {
        Store::connect_with(
            self.config.clone(),
            self.keys(),
            Arc::new(self.objects.clone()),
        )
        .await
    }

    pub(crate) async fn drop_schema(self) {
        self.admin
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .await
            .unwrap();
        drop(self.admin);
        self.connection.await.unwrap();
    }
}
