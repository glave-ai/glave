//! The person's controls over memory through the session API: `memory/list`, `save`, `edit`,
//! `move` and `forget`, over JSON as in-process; another person may not touch the person's own
//! memory; and a server without memory says so.

use std::sync::Arc;

use glave_event::{MemoryScope, PrincipalId};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_memory::{GlaveMemory, Memories, Personal, Rules};
use glave_ports::testing::TempDir;
use glave_ports::{Caller, OrgId, ProjectId, Scope};
use glave_session::Answering;
use glave_session::protocol::{Reply, Request, code};
use glave_store_sqlite::{SqliteLog, Store};

use crate::support::*;

fn person() -> PrincipalId {
    PrincipalId::try_from(uuid::Uuid::from_u128(
        0x0192_f5c0_0000_7000_8000_0000_0000_0032,
    ))
    .unwrap()
}

fn personal() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        ProjectId::from_uuid(uuid::Uuid::from_u128(100)),
    )
}

async fn served(name: &str) -> (Tested<SqliteLog>, Store, TempDir) {
    let store =
        Store::open_in_memory(KeySettings::new(Arc::new(InMemoryRoot::new().unwrap()))).unwrap();
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
        Box::new(glave_ports::SystemClock),
        Box::new(glave_ports::OsEntropy),
    );
    let mut setup = Setup::new(Vec::new());
    setup.answering = Answering::NoOne;
    setup.settings.person = Some(person());
    setup.memory = Some(Arc::new(GlaveMemory::start(memories).await.unwrap()));
    let dir = folder(name);
    let (server, _) = setup.build_on(&dir, store.log()).await;
    (server, store, dir)
}

fn listed(reply: Reply) -> Vec<glave_ports::memories::Remembered> {
    match reply {
        Reply::Memories { memories } => memories,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_person_lists_saves_edits_moves_and_forgets_their_memory() {
    block_on!(async {
        let (server, _store, _dir) = served("memory-controls").await;
        // Over JSON, as any transport carries it.
        let save: Request = serde_json::from_value(serde_json::json!({
            "method": "memory/save",
            "params": {"text": "Ada prefers short answers", "scope": "project"}
        }))
        .unwrap();
        let saved = listed(server.request(save).await.unwrap());
        let id = saved[0].memory;
        let edited = listed(
            server
                .request(Request::MemoryEdit {
                    memory: id,
                    text: "Ada prefers very short answers".into(),
                    project: None,
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
                    project: None,
                })
                .await
                .unwrap(),
        );
        assert_eq!(moved[0].scope, MemoryScope::Personal);
        let all = listed(
            server
                .request(Request::MemoryList {
                    scope: None,
                    project: None,
                })
                .await
                .unwrap(),
        );
        assert_eq!(all.len(), 1);
        let forgot = server
            .request(Request::MemoryForget {
                memory: Some(moved[0].memory),
                all: false,
                scope: None,
                project: None,
            })
            .await;
        assert!(forgot.is_ok(), "{forgot:?}");
        let left = server
            .request(Request::MemoryList {
                scope: None,
                project: None,
            })
            .await;
        assert_eq!(listed(left.unwrap()).len(), 0);
        let missing = server
            .request(Request::MemoryRestore {
                memory: id,
                project: None,
            })
            .await
            .unwrap_err();
        assert_eq!(missing.code, code::NO_MEMORY);
    });
}

#[test]
fn a_request_that_names_a_project_acts_in_that_project_s_memory() {
    block_on!(async {
        let (server, _store, _dir) = served("memory-projects").await;
        let created = Request::ProjectCreate {
            name: "other".into(),
            members: Vec::new(),
            folder: None,
        };
        let Ok(Reply::Project { project: other, .. }) = server.request(created).await else {
            panic!("project/create");
        };
        let save = Request::MemorySave {
            text: "the other project ships on Fridays".into(),
            scope: MemoryScope::Project,
            project: Some(other.id),
        };
        assert_eq!(listed(server.request(save).await.unwrap()).len(), 1);
        let list = |project| Request::MemoryList {
            scope: None,
            project,
        };
        let here = listed(server.request(list(None)).await.unwrap());
        assert!(here.is_empty(), "not the caller's Chats': {here:?}");
        let there = listed(server.request(list(Some(other.id))).await.unwrap());
        assert_eq!(there.len(), 1);
        assert_eq!(there[0].text, "the other project ships on Fridays");
    });
}

#[test]
fn a_request_that_names_no_project_acts_in_the_caller_s_chats_never_the_folder_s_project() {
    block_on!(async {
        let (server, _store, _dir) = served("memory-chats").await;
        let save = Request::MemorySave {
            text: "Ada keeps her notes in Chats".into(),
            scope: MemoryScope::Project,
            project: None,
        };
        assert_eq!(listed(server.request(save).await.unwrap()).len(), 1);
        let list = |project| Request::MemoryList {
            scope: Some(MemoryScope::Project),
            project,
        };
        let chats = listed(server.request(list(None)).await.unwrap());
        assert_eq!(chats.len(), 1, "{chats:?}");
        let folder = listed(server.request(list(Some(scope().project()))).await.unwrap());
        assert!(
            folder.is_empty(),
            "the folder's project has none: {folder:?}"
        );
    });
}

#[test]
fn another_person_may_not_change_the_persons_own_memory() {
    block_on!(async {
        let (server, _store, _dir) = served("memory-others").await;
        let someone = Caller {
            person: Some(
                PrincipalId::try_from(uuid::Uuid::from_u128(
                    0x0192_f5c0_0000_7000_8000_0000_0000_0099,
                ))
                .unwrap(),
            ),
            auth: None,
            trace: None,
        };
        let refused = server
            .request_as(
                &someone,
                Request::MemorySave {
                    text: "they like tea".into(),
                    scope: MemoryScope::Personal,
                    project: None,
                },
            )
            .await
            .unwrap_err();
        // It acts in their own Chats, and the person's own memory is still not theirs.
        assert_eq!(refused.code, code::NOT_ALLOWED);
    });
}

#[test]
fn a_server_without_memory_says_so() {
    block_on!(async {
        let dir = folder("memory-off");
        let server = server(&dir, Vec::new(), Answering::NoOne).await;
        let off = server
            .request(Request::MemoryList {
                scope: None,
                project: None,
            })
            .await
            .unwrap_err();
        assert_eq!(off.code, code::MEMORY_OFF);
    });
}
