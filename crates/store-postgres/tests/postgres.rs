//! The Postgres adapters against a real database. They run when `GLAVE_TEST_POSTGRES_URL`
//! names one (CI sets it) and are skipped otherwise. Each test works in a schema of its own
//! and drops it at the end, so tests run in parallel and leave nothing behind.

#[cfg(test)]
mod postgres {
    use glave_event::{At, EventId, Header, Outcome, RawEvent, Seq, ThreadId, TurnEnded, encode};
    use glave_ports::{BlobStore, EventLog, OrgId, Scope, SpaceId, conformance, digest};
    use glave_store_postgres::{Config, Store, StoreError};
    use tokio::task::JoinHandle;
    use tokio_postgres::{Client, NoTls};
    use uuid::{Builder, Uuid};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn scope() -> Scope {
        Scope {
            org: OrgId::from_uuid(Uuid::from_u128(1)),
            space: SpaceId::from_uuid(Uuid::from_u128(2)),
        }
    }

    /// The first event of a thread, as each racing writer would write it.
    fn first_event(thread: ThreadId, writer: u8) -> RawEvent {
        let id = Builder::from_unix_timestamp_millis(1_791_000_000_000, &[writer; 10]).into_uuid();
        let header = Header {
            id: EventId::try_from(id).unwrap(),
            thread,
            seq: Seq::FIRST,
            at: At::try_from("2026-10-10T09:00:00.000Z".to_owned()).unwrap(),
        };
        let ended = TurnEnded {
            outcome: Outcome::Completed,
        };
        encode(header, &ended).unwrap()
    }

    /// A schema of the test's own, and an admin connection to make and drop it.
    struct Scratch {
        admin: Client,
        connection: JoinHandle<()>,
        schema: String,
        config: Config,
    }

    impl Scratch {
        /// A fresh, empty schema, or `None` when no database was given.
        async fn new(name: &str) -> Option<Self> {
            let Ok(url) = std::env::var("GLAVE_TEST_POSTGRES_URL") else {
                eprintln!("skipped: set GLAVE_TEST_POSTGRES_URL to run the Postgres tests");
                return None;
            };
            let (admin, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
            let connection = tokio::spawn(async move {
                let _ = connection.await;
            });
            // Test processes run one test each, so the process id keeps parallel runs apart.
            let schema = format!("glave_test_{name}_{}", std::process::id());
            admin
                .batch_execute(&format!(
                    "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema};"
                ))
                .await
                .unwrap();
            let mut config: Config = url.parse().unwrap();
            config.options(format!("-c search_path={schema}"));
            Some(Self {
                admin,
                connection,
                schema,
                config,
            })
        }

        async fn store(&self) -> Result<Store, StoreError> {
            Store::connect_with(self.config.clone()).await
        }

        async fn drop_schema(self) {
            self.admin
                .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema))
                .await
                .unwrap();
            drop(self.admin);
            self.connection.await.unwrap();
        }
    }

    #[test]
    fn the_log_passes_the_event_log_conformance_suite() {
        block_on(async {
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

    #[test]
    fn the_blobs_pass_the_blob_store_conformance_suite() {
        block_on(async {
            let Some(scratch) = Scratch::new("blobs").await else {
                return;
            };
            let store = scratch.store().await.unwrap();
            let verdict = conformance::blob_store(&store.blobs()).await;
            drop(store);
            scratch.drop_schema().await;
            assert_eq!(verdict, Ok(()));
        });
    }

    #[test]
    fn data_survives_reconnecting_and_migrations_run_once() {
        block_on(async {
            let Some(scratch) = Scratch::new("reconnect").await else {
                return;
            };
            let scope = scope();
            let first = scratch.store().await.unwrap();
            first
                .blobs()
                .put(&scope, b"kept".to_vec(), "text/plain")
                .await
                .unwrap();
            drop(first);
            let again = scratch.store().await.unwrap();
            let kept = again.blobs().get(&scope, &digest(b"kept")).await;
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
            assert_eq!(kept, Ok(Some(b"kept".to_vec())));
            assert_eq!(versions, 1, "each migration is recorded once");
        });
    }

    #[test]
    fn openers_racing_on_a_new_database_migrate_it_once() {
        block_on(async {
            let Some(scratch) = Scratch::new("race").await else {
                return;
            };
            let openers: Vec<_> = (0..4)
                .map(|_| tokio::spawn(Store::connect_with(scratch.config.clone())))
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
    fn writers_racing_for_one_position_leave_exactly_one_winner() {
        block_on(async {
            let Some(scratch) = Scratch::new("writers").await else {
                return;
            };
            let store = scratch.store().await.unwrap();
            let thread = ThreadId::try_from(
                Builder::from_unix_timestamp_millis(1_791_000_000_000, &[9; 10]).into_uuid(),
            )
            .unwrap();
            let writers: Vec<_> = (0..8)
                .map(|writer| {
                    let log = store.log();
                    let event = first_event(thread, writer);
                    tokio::spawn(async move { log.append(&scope(), &[event]).await })
                })
                .collect();
            let mut won = 0;
            for writer in writers {
                if writer.await.unwrap().is_ok() {
                    won += 1;
                }
            }
            let stored = store.log().read(&scope(), thread, None, 100).await;
            drop(store);
            scratch.drop_schema().await;
            assert_eq!(won, 1, "exactly one writer takes the position");
            assert_eq!(stored.map(|events| events.len()), Ok(1));
        });
    }

    #[test]
    fn a_newer_schema_is_refused() {
        block_on(async {
            let Some(scratch) = Scratch::new("newer").await else {
                return;
            };
            drop(scratch.store().await.unwrap());
            scratch
                .admin
                .execute(
                    &format!(
                        "INSERT INTO {}.glave_migrations (version) VALUES (999)",
                        scratch.schema
                    ),
                    &[],
                )
                .await
                .unwrap();
            let result = scratch.store().await.map(drop);
            scratch.drop_schema().await;
            assert!(
                matches!(&result, Err(StoreError::Schema(message)) if message.contains("newer glave")),
                "{result:?}"
            );
        });
    }
}
