//! A database carried forward (`specs/storage.md`, "Migrations"): from each earlier version, with
//! what it held; by one of the servers starting on it at once; refused, unchanged, when a newer
//! glave set it up or it is from before the baseline; and left at its version when a migration
//! fails. They run where `GLAVE_TEST_POSTGRES_URL` names a database (CI) and are skipped
//! otherwise.

use std::sync::Arc;

use glave_ports::conformance::{StoreSuite, store_suite};
use glave_ports::migrations::{Change, Migration, Schema};
use glave_store_postgres::{SCHEMA, Store, StoreError, migrate};
use tokio_postgres::NoTls;

use crate::support::{Scratch, block_on};

/// Every version a database glave makes records, in order.
pub(crate) fn every_version() -> Vec<i32> {
    (SCHEMA.baseline..=SCHEMA.latest())
        .map(|version| i32::try_from(version).unwrap())
        .collect()
}

/// Brings the scratch schema up to `schema` as the store's role, outside any store.
async fn migrated(scratch: &Scratch, schema: &Schema<'_>) -> Result<(), StoreError> {
    let (mut client, connection) = scratch.config.connect(NoTls).await.unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let migrated = migrate(&mut client, schema).await;
    drop(client);
    connection.await.unwrap();
    migrated
}

/// The scratch schema at `version` of glave's, made by its own migrations, holding a setting.
async fn at(scratch: &Scratch, version: u32) {
    let count = usize::try_from(version - SCHEMA.baseline).unwrap();
    let earlier = Schema {
        migrations: &SCHEMA.migrations[..count],
        ..SCHEMA
    };
    migrated(scratch, &earlier).await.unwrap();
    let sql = format!(
        "INSERT INTO {}.glave_settings (key, value) VALUES ('kept', 'yes')",
        scratch.schema
    );
    scratch.admin.execute(&sql, &[]).await.unwrap();
}

/// Each earlier version, built from the recorded migrations, opens at the latest and passes
/// every suite of a whole store, each on a schema of its own.
#[test]
fn a_database_of_each_earlier_version_is_brought_up_to_date_keeping_what_it_held() {
    for version in SCHEMA.baseline..SCHEMA.latest() {
        for suite in StoreSuite::ALL {
            block_on!(async {
                let name = format!("from_{version}_{suite:?}").to_lowercase();
                let Some(scratch) = Scratch::new(&name).await else {
                    return;
                };
                at(&scratch, version).await;
                let store = scratch.store().await.unwrap();
                let (log, projects) = (store.log(), store.projects());
                let (directory, leases) = (store.directory(), store.leases());
                let verdict = store_suite(suite, &log, &projects, &directory, &leases).await;
                drop((log, projects, directory, leases, store));
                let versions = scratch.versions().await;
                let sql = format!(
                    "SELECT value FROM {}.glave_settings WHERE key = 'kept'",
                    scratch.schema
                );
                let kept: String = scratch.admin.query_one(&sql, &[]).await.unwrap().get(0);
                scratch.drop_schema().await;
                assert_eq!(verdict, Ok(()), "from {version}: {suite:?}");
                assert_eq!(versions, every_version());
                assert_eq!(
                    kept, "yes",
                    "from {version}, what the database held is kept"
                );
            });
        }
    }
}

/// Servers starting at once on a new database, and on one of the baseline: one migrates it,
/// the others wait for it and find it done, so every version is made and recorded once.
#[test]
fn servers_starting_at_once_on_one_database_migrate_it_once() {
    for from in [None, Some(SCHEMA.baseline)] {
        block_on!(async {
            let name = format!("racing_{}", from.unwrap_or(0));
            let Some(scratch) = Scratch::new(&name).await else {
                return;
            };
            if let Some(version) = from {
                at(&scratch, version).await;
            }
            let starting: Vec<_> = (0..4)
                .map(|_| {
                    let objects = Arc::new(scratch.objects.clone());
                    tokio::spawn(Store::connect_with(
                        scratch.config.clone(),
                        scratch.keys(),
                        objects,
                    ))
                })
                .collect();
            let mut results = Vec::new();
            for server in starting {
                results.push(server.await.unwrap().map(drop));
            }
            let versions = scratch.versions().await;
            scratch.drop_schema().await;
            for result in results {
                assert!(result.is_ok(), "from {from:?}: {result:?}");
            }
            assert_eq!(versions, every_version(), "from {from:?}");
        });
    }
}

/// A database a newer glave set up is never downgraded, and one from before the baseline is
/// never guessed at: each is refused with what to do, and left as it was.
#[test]
fn a_database_a_newer_glave_set_up_or_from_before_the_baseline_is_refused_unchanged() {
    block_on!(async {
        let Some(scratch) = Scratch::new("refused").await else {
            return;
        };
        drop(scratch.store().await.unwrap());
        let migrations = format!("{}.glave_migrations", scratch.schema);
        let cases = [
            (
                format!(
                    "INSERT INTO {migrations} (version) VALUES ({})",
                    SCHEMA.latest() + 1
                ),
                "Update glave",
            ),
            (
                format!(
                    "DELETE FROM {migrations}; INSERT INTO {migrations} (version) VALUES ({})",
                    SCHEMA.baseline - 1
                ),
                "Empty it",
            ),
        ];
        let mut seen = Vec::new();
        for (recorded, what) in cases {
            scratch.admin.batch_execute(&recorded).await.unwrap();
            let before = scratch.versions().await;
            let result = scratch.store().await.map(drop);
            seen.push((what, result, before == scratch.versions().await));
        }
        scratch.drop_schema().await;
        for (what, result, unchanged) in seen {
            assert!(
                matches!(&result, Err(StoreError::Schema(message)) if message.contains(what)),
                "{result:?}"
            );
            assert!(unchanged, "{what}: the database is not changed");
        }
    });
}

/// A migration that fails undoes every step of its transaction, the ones before it too, and
/// leaves the database at its version, which a later start brings forward.
#[test]
fn a_failed_migration_leaves_the_database_at_its_version_with_none_of_its_steps() {
    block_on!(async {
        let Some(scratch) = Scratch::new("failed").await else {
            return;
        };
        at(&scratch, SCHEMA.baseline).await;
        let mut failing = SCHEMA.migrations.to_vec();
        failing.push(Migration {
            change: Change::Adds,
            sql: "CREATE TABLE tags (name text); SELECT * FROM missing;",
        });
        let schema = Schema {
            migrations: &failing,
            ..SCHEMA
        };
        let failed = migrated(&scratch, &schema).await;
        let versions = scratch.versions().await;
        let sql = format!(
            "SELECT to_regclass('{0}.tags') IS NULL AND to_regclass('{0}.event_search') IS NULL",
            scratch.schema
        );
        let undone: bool = scratch.admin.query_one(&sql, &[]).await.unwrap().get(0);
        let later = scratch.store().await.map(drop);
        let after = scratch.versions().await;
        scratch.drop_schema().await;
        let expected = format!(
            "could not bring the database up to date (schema {})",
            SCHEMA.latest() + 1
        );
        assert!(
            matches!(&failed, Err(StoreError::Schema(message)) if message.starts_with(&expected)),
            "{failed:?}"
        );
        let baseline = i32::try_from(SCHEMA.baseline).unwrap();
        assert_eq!(versions, [baseline], "at the version it had");
        assert!(undone, "no step of the failed transaction is left");
        assert!(later.is_ok(), "{later:?}");
        assert_eq!(after, every_version());
    });
}
