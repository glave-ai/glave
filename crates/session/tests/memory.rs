//! The person's controls over memory through the session API: `memory/list`, `save`, `edit`,
//! `move` and `forget`, over JSON as in-process; another person may not touch the person's own
//! memory; and a server without memory says so.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod memory {
    use std::sync::Arc;

    use glave_event::{MemoryScope, PrincipalId};
    use glave_harness::{Caller, Settings};
    use glave_keys::{KeySettings, MemoryRoot};
    use glave_memory::{GlaveMemory, Memories, Personal, Rules};
    use glave_ports::{OrgId, Scope, SpaceId};
    use glave_session::protocol::{Reply, Request, code};
    use glave_store_sqlite::Store;

    use super::support::*;

    fn person() -> PrincipalId {
        PrincipalId::try_from(uuid::Uuid::from_u128(
            0x0192_f5c0_0000_7000_8000_0000_0000_0032,
        ))
        .unwrap()
    }

    fn personal() -> Scope {
        Scope {
            org: OrgId::from_uuid(uuid::Uuid::from_u128(1)),
            space: SpaceId::from_uuid(uuid::Uuid::from_u128(100)),
        }
    }

    async fn served(
        name: &str,
    ) -> (
        glave_session::Server<Script, glave_store_sqlite::SqliteLog, glave_ports::MemoryBlobs>,
        Store,
    ) {
        let store =
            Store::open_in_memory(KeySettings::new(Arc::new(MemoryRoot::new().unwrap()))).unwrap();
        let rules = Rules {
            personal: Some(Personal {
                scope: personal(),
                person: person(),
            }),
            ..Rules::default()
        };
        let memories = Memories::new(
            store.log(),
            store.keyring(),
            rules,
            Box::new(glave_harness::SystemClock),
            Box::new(glave_harness::OsEntropy),
        );
        let memory = Arc::new(GlaveMemory::start(memories).await.unwrap());
        let mut settings = Settings::new("test-model", "script");
        settings.person = Some(person());
        let server = with_memory(&folder(name), store.log(), memory, settings).await;
        (server, store)
    }

    fn listed(reply: Reply) -> Vec<glave_harness::Remembered> {
        match reply {
            Reply::Memories { memories } => memories,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_person_lists_saves_edits_moves_and_forgets_their_memory() {
        block_on(async {
            let (server, _store) = served("memory-controls").await;
            // Over JSON, as any transport carries it.
            let save: Request = serde_json::from_value(serde_json::json!({
                "method": "memory/save",
                "params": {"text": "Ada prefers short answers", "scope": "space"}
            }))
            .unwrap();
            let saved = listed(server.request(save).await.unwrap());
            let id = saved[0].memory;
            let edited = listed(
                server
                    .request(Request::MemoryEdit {
                        memory: id,
                        text: "Ada prefers very short answers".into(),
                    })
                    .await
                    .unwrap(),
            );
            assert_eq!(edited[0].version, 2);
            let moved = listed(
                server
                    .request(Request::MemoryMove {
                        memory: id,
                        scope: MemoryScope::Personal,
                    })
                    .await
                    .unwrap(),
            );
            assert_eq!(moved[0].scope, MemoryScope::Personal);
            let all = listed(
                server
                    .request(Request::MemoryList { scope: None })
                    .await
                    .unwrap(),
            );
            assert_eq!(all.len(), 1);
            let forgot = server
                .request(Request::MemoryForget {
                    memory: Some(moved[0].memory),
                    all: false,
                    scope: None,
                })
                .await;
            assert!(forgot.is_ok(), "{forgot:?}");
            assert!(
                listed(
                    server
                        .request(Request::MemoryList { scope: None })
                        .await
                        .unwrap()
                )
                .is_empty()
            );
            let missing = server
                .request(Request::MemoryRestore { memory: id })
                .await
                .unwrap_err();
            assert_eq!(missing.code, code::NO_MEMORY);
        });
    }

    #[test]
    fn another_person_may_not_change_the_persons_own_memory() {
        block_on(async {
            let (server, _store) = served("memory-others").await;
            let someone = Caller {
                person: Some(
                    PrincipalId::try_from(uuid::Uuid::from_u128(
                        0x0192_f5c0_0000_7000_8000_0000_0000_0099,
                    ))
                    .unwrap(),
                ),
                auth: None,
            };
            let refused = server
                .request_as(
                    &someone,
                    Request::MemorySave {
                        text: "they like tea".into(),
                        scope: MemoryScope::Personal,
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(refused.code, code::NOT_ALLOWED);
        });
    }

    #[test]
    fn a_server_without_memory_says_so() {
        block_on(async {
            let dir = folder("memory-off");
            let server = server(&dir, Vec::new(), glave_session::Answering::NoOne).await;
            let off = server
                .request(Request::MemoryList { scope: None })
                .await
                .unwrap_err();
            assert_eq!(off.code, code::MEMORY_OFF);
        });
    }
}
