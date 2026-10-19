//! The Postgres adapters against a real database. They run when `GLAVE_TEST_POSTGRES_URL`
//! names one (CI sets it) and are skipped otherwise. Each test works in a schema of its own
//! and drops it at the end, so tests run in parallel and leave nothing behind.

#[cfg(test)]
#[path = "support/scratch.rs"]
mod support;

#[cfg(test)]
mod postgres {
    use glave_event::{At, EventId, Header, Outcome, RawEvent, Seq, ThreadId, TurnEnded, encode};
    use glave_identity::signin::Browsers;
    use glave_ports::{BlobStore, EventLog, OrgId, ProjectId, Scope, conformance, digest};
    use glave_store_postgres::{Store, StoreError};
    use uuid::{Builder, Uuid};

    use super::support::{Scratch, block_on};

    fn scope() -> Scope {
        Scope {
            org: OrgId::from_uuid(Uuid::from_u128(1)),
            project: ProjectId::from_uuid(Uuid::from_u128(2)),
        }
    }

    fn thread() -> ThreadId {
        ThreadId::try_from(
            Builder::from_unix_timestamp_millis(1_791_000_000_000, &[9; 10]).into_uuid(),
        )
        .unwrap()
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
            // As after the upgrade that made the table: no views, and no version recorded.
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
            assert_eq!(kept.len(), 4);
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
    fn the_blobs_pass_the_blob_store_conformance_suite() {
        block_on!(async {
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
        block_on!(async {
            let Some(scratch) = Scratch::new("reconnect").await else {
                return;
            };
            let scope = scope();
            let first = scratch.store().await.unwrap();
            first
                .blobs()
                .put(&scope, thread(), b"kept".to_vec(), "text/plain")
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
            assert_eq!(versions, 5, "each migration is recorded once");
        });
    }

    #[test]
    fn openers_racing_on_a_new_database_migrate_it_once() {
        block_on!(async {
            let Some(scratch) = Scratch::new("race").await else {
                return;
            };
            let openers: Vec<_> = (0..4)
                .map(|_| tokio::spawn(Store::connect_with(scratch.config.clone(), scratch.keys())))
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
        block_on!(async {
            let Some(scratch) = Scratch::new("writers").await else {
                return;
            };
            let store = scratch.store().await.unwrap();
            let thread = thread();
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
    fn servers_on_one_database_racing_to_make_the_identity_all_get_the_first() {
        block_on!(async {
            let Some(scratch) = Scratch::new("identity").await else {
                return;
            };
            let principal = |n: u8| {
                glave_event::PrincipalId::try_from(
                    uuid::Builder::from_unix_timestamp_millis(1, &[n; 10]).into_uuid(),
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
                |store: &Store| Browsers::open(store.log(), scope(), server.to_string(), clock());
            let (one, two) = (open(&first).await.unwrap(), open(&second).await.unwrap());
            let session = one.redeem(one.link().unwrap().expose()).await.unwrap();
            let verified = two.verify(session.token.expose()).await;
            two.end(&session.id).await.unwrap();
            let ended = open(&first)
                .await
                .unwrap()
                .verify(session.token.expose())
                .await;
            drop((one, two, first, second));
            scratch.drop_schema().await;
            assert_eq!(verified.unwrap(), session.id);
            assert!(ended.is_err(), "signed out on the other server");
        });
    }

    #[test]
    fn a_newer_schema_is_refused() {
        block_on!(async {
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
