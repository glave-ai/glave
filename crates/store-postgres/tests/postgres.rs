//! The Postgres adapters against a real database. They run when `GLAVE_TEST_POSTGRES_URL`
//! names one (CI sets it) and are skipped otherwise. Each test works in a schema of its own
//! and drops it at the end, so tests run in parallel and leave nothing behind.

use std::sync::Arc;

use glave_event::ThreadId;
use glave_identity::signin::Browsers;
use glave_ports::{BlobStore, OrgId, Pieces, ProjectId, Scope, conformance, digest, read_all};
use glave_store_postgres::{Store, StoreError};
use uuid::{Builder, Uuid};

use crate::support::{Scratch, block_on};

fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(Uuid::from_u128(1)),
        ProjectId::from_uuid(Uuid::from_u128(2)),
    )
}

fn thread() -> ThreadId {
    ThreadId::try_from(Builder::from_unix_timestamp_millis(1_791_000_000_000, &[9; 10]).into_uuid())
        .unwrap()
}

#[test]
fn the_projects_pass_the_projects_conformance_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("projects").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let verdict = conformance::projects(&store.projects()).await;
        drop(store);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
    });
}

#[test]
fn the_log_passes_the_event_log_conformance_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("log").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let verdict = conformance::event_log(&store.log()).await;
        drop(store);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
    });
}

/// Every row of the threads view, by thread.
async fn rows(scratch: &Scratch) -> Vec<(String, String)> {
    let sql = format!(
        "SELECT thread::text, view FROM {}.threads ORDER BY thread",
        scratch.schema
    );
    scratch
        .admin
        .query(&sql, &[])
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect()
}

#[test]
fn the_log_passes_the_threads_view_conformance_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("views").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let verdict = conformance::thread_views(&store.log()).await;
        let kept = rows(&scratch).await;
        // As if no fold had built them: no views, and no version recorded.
        scratch
            .admin
            .batch_execute(&format!(
                "DELETE FROM {0}.threads; DELETE FROM {0}.glave_settings;",
                scratch.schema
            ))
            .await
            .unwrap();
        drop(store);
        let reconnected = scratch.store().await.unwrap();
        let refilled = rows(&scratch).await;
        drop(reconnected);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
        assert_eq!(kept.len(), 7);
        assert_eq!(refilled, kept, "connecting fills the view from the log");
    });
}

#[test]
fn the_log_passes_the_search_conformance_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("search").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let verdict = conformance::search(&store.log()).await;
        drop(store);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
    });
}

#[test]
fn the_leases_pass_the_leases_conformance_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("leases").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let verdict = conformance::leases(&store.leases()).await;
        drop(store);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
    });
}

/// Servers on one database, each through its own pool, racing for one lease: one gets it, the
/// rest see it held until it is let go.
#[test]
fn servers_on_one_database_racing_for_a_lease_leave_it_with_one() {
    use glave_ports::{Holder, Leased, Leases as _};
    block_on!(async {
        let Some(scratch) = Scratch::new("racing_leases").await else {
            return;
        };
        let mut servers = Vec::new();
        for _ in 0..4 {
            servers.push(scratch.store().await.unwrap().leases());
        }
        let org = OrgId::from_uuid(Uuid::from_u128(1));
        let hour = std::time::Duration::from_secs(3_600);
        let racing: Vec<_> = servers
            .iter()
            .enumerate()
            .map(|(n, leases)| {
                let leases = leases.clone();
                let holder = Holder::from_uuid(Uuid::from_u128(10 + n as u128));
                tokio::spawn(
                    async move { leases.acquire(org, Leased::Scheduler, holder, hour).await },
                )
            })
            .collect();
        let mut won = Vec::new();
        for race in racing {
            if let Some(lease) = race.await.unwrap().unwrap() {
                won.push(lease);
            }
        }
        let late = Holder::from_uuid(Uuid::from_u128(99));
        let held = servers[0].acquire(org, Leased::Scheduler, late, hour).await;
        if let Some(lease) = won.first() {
            servers[1].release(lease).await.unwrap();
        }
        let after = servers[2].acquire(org, Leased::Scheduler, late, hour).await;
        drop(servers);
        scratch.drop_schema().await;
        assert_eq!(won.len(), 1, "{won:?}");
        assert_eq!(held, Ok(None));
        assert!(after.unwrap().is_some(), "let go, it is anyone's");
    });
}

#[test]
fn the_blobs_pass_the_blob_store_conformance_suite() {
    block_on!(async {
        let Some(scratch) = Scratch::new("blobs").await else {
            return;
        };
        let store = scratch.store().await.unwrap();
        let verdict =
            conformance::blob_store(&store.blobs(), &scratch.clock, Some(&scratch.objects)).await;
        drop(store);
        scratch.drop_schema().await;
        assert_eq!(verdict, Ok(()));
    });
}

#[test]
fn data_survives_reconnecting_and_the_schema_is_recorded_once() {
    block_on!(async {
        let Some(scratch) = Scratch::new("reconnect").await else {
            return;
        };
        let scope = scope();
        let first = scratch.store().await.unwrap();
        first
            .blobs()
            .put(
                &scope,
                Some(thread()),
                "text/plain",
                4,
                Box::new(Pieces::from(b"kept".to_vec())),
            )
            .await
            .unwrap();
        drop(first);
        let again = scratch.store().await.unwrap();
        let mut kept = again
            .blobs()
            .get(&scope, &digest(b"kept"))
            .await
            .unwrap()
            .unwrap();
        let kept = read_all(kept.bytes.as_mut(), 4).await;
        drop(again);
        let versions = scratch
            .admin
            .query_one(
                &format!("SELECT COUNT(*) FROM {}.glave_migrations", scratch.schema),
                &[],
            )
            .await
            .unwrap()
            .get::<_, i64>(0);
        scratch.drop_schema().await;
        assert_eq!(kept, Ok(b"kept".to_vec()));
        assert_eq!(versions, 1, "the schema is recorded once");
    });
}

#[test]
fn openers_racing_on_a_new_database_make_its_schema_once() {
    block_on!(async {
        let Some(scratch) = Scratch::new("race").await else {
            return;
        };
        let openers: Vec<_> = (0..4)
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
        for opener in openers {
            results.push(opener.await.unwrap().map(drop));
        }
        scratch.drop_schema().await;
        for result in results {
            assert!(result.is_ok(), "{result:?}");
        }
    });
}

#[test]
fn servers_on_one_database_racing_to_make_the_identity_all_get_the_first() {
    block_on!(async {
        let Some(scratch) = Scratch::new("identity").await else {
            return;
        };
        let principal = |n: u8| {
            glave_event::PrincipalId::try_from(
                Builder::from_unix_timestamp_millis(1, &[n; 10]).into_uuid(),
            )
            .unwrap()
        };
        let mut servers = Vec::new();
        for _ in 0..4 {
            servers.push(scratch.store().await.unwrap());
        }
        let racing: Vec<_> = servers
            .iter()
            .zip(0u8..)
            .map(|(store, n)| {
                let store = store.clone();
                let new = (principal(2 * n + 1), principal(2 * n + 2));
                tokio::spawn(async move { store.identity(new).await })
            })
            .collect();
        let mut seen = Vec::new();
        for server in racing {
            seen.push(server.await.unwrap().unwrap());
        }
        let rows = scratch
            .admin
            .query(
                &format!(
                    "SELECT key FROM {}.glave_settings WHERE key IN ('person', 'agent')",
                    scratch.schema
                ),
                &[],
            )
            .await
            .unwrap()
            .len();
        drop(servers);
        scratch.drop_schema().await;
        let first = seen[0];
        assert!(seen.iter().all(|identity| *identity == first), "{seen:?}");
        assert_ne!(
            first.0, first.1,
            "the person and the agent are kept as a pair"
        );
        assert_eq!(rows, 2, "one person and one agent");
    });
}

#[test]
fn a_browser_signed_in_on_one_server_is_signed_in_on_another_on_the_database() {
    block_on!(async {
        let Some(scratch) = Scratch::new("browsers").await else {
            return;
        };
        let (first, second) = (
            scratch.store().await.unwrap(),
            scratch.store().await.unwrap(),
        );
        let server = first.server(Uuid::from_u128(7)).await.unwrap();
        assert_eq!(second.server(Uuid::from_u128(8)).await.unwrap(), server);
        let clock = || Box::new(glave_ports::testing::ManualClock::at_second(1_791_590_400));
        let open =
            |store: &Store| Browsers::open(store.log(), scope(), server.to_string(), None, clock());
        let (one, two) = (open(&first).await.unwrap(), open(&second).await.unwrap());
        let session = one.redeem(one.link().unwrap().expose()).await.unwrap();
        let verified = two.verify(session.token.expose(), true).await;
        two.end(&session.id).await.unwrap();
        let ended = open(&first)
            .await
            .unwrap()
            .verify(session.token.expose(), true)
            .await;
        drop((one, two, first, second));
        scratch.drop_schema().await;
        assert_eq!(verified.unwrap(), session.id);
        assert!(ended.is_err(), "signed out on the other server");
    });
}

/// A database another version of glave set up, older or newer, is refused with what to
/// do: until glave 0.1 there is one schema and no migration.
#[test]
fn a_database_of_another_schema_is_refused() {
    block_on!(async {
        let Some(scratch) = Scratch::new("other").await else {
            return;
        };
        drop(scratch.store().await.unwrap());
        let mut results = Vec::new();
        for found in [6, 999] {
            scratch
                .admin
                .execute(
                    &format!(
                        "UPDATE {}.glave_migrations SET version = $1",
                        scratch.schema
                    ),
                    &[&found],
                )
                .await
                .unwrap();
            results.push(scratch.store().await.map(drop));
        }
        scratch.drop_schema().await;
        for result in results {
            assert!(
                matches!(&result, Err(StoreError::Schema(message)) if message.contains("Empty it")),
                "{result:?}"
            );
        }
    });
}
