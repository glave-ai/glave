//! The person's controls over memory through the session API: `memory/list`, `save`, `edit`,
//! `move` and `forget`, over JSON as in-process; another person may not touch the person's own
//! memory; and a server without memory says so.

use std::sync::Arc;

use glave_event::{Body, MemoryScope, PrincipalId, PrincipalKind, decode};
use glave_keys::{InMemoryRoot, KeySettings};
use glave_memory::{GlaveMemory, Memories, Personal, Rules};
use glave_ports::memories::{Memory, RECALL, REMEMBER};
use glave_ports::stream::Reserved;
use glave_ports::testing::{CountingEntropy, ManualClock, TempDir};
use glave_ports::{Caller, EventLog, OrgId, ProjectId, Scope};
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
    served_with(name, Vec::new()).await
}

/// A server with memory whose model answers with `answers`.
async fn served_with(
    name: &str,
    answers: Vec<glave_ports::Completion>,
) -> (Tested<SqliteLog>, Store, TempDir) {
    let (server, store, dir, _) = served_seeing(name, answers).await;
    (server, store, dir)
}

/// [`served_with`], and every request its model receives.
pub(crate) async fn served_seeing(
    name: &str,
    answers: Vec<glave_ports::Completion>,
) -> (Tested<SqliteLog>, Store, TempDir, Seen) {
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
    let mut setup = Setup::new(answers);
    setup.answering = Answering::NoOne;
    setup.settings.person = Some(person());
    setup.memory = Some(Arc::new(GlaveMemory::start(memories).await.unwrap()));
    let dir = folder(name);
    let (server, seen) = setup.build_on(&dir, store.log()).await;
    (server, store, dir, seen)
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

/// Someone other than the person the server works for, signed in through the company login: a
/// caller with no sign-in at all reads as the person on this computer.
fn someone() -> Caller {
    signed_in("someone")
}

#[test]
fn another_person_may_not_change_the_persons_own_memory() {
    block_on!(async {
        let (server, _store, _dir) = served("memory-others").await;
        let someone = someone();
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

/// What the model answers to save `text` to the person's memory, then say so.
fn saving(text: &str) -> Vec<glave_ports::Completion> {
    let save = serde_json::json!({"text": text, "scope": "personal"});
    vec![
        answer(None, Some((REMEMBER, save))),
        answer(Some("Saved."), None),
    ]
}

/// Another person's model, driven to it by a page it read, reaches nothing of the person's
/// through memory's own tools: it recalls neither what they had glave remember nor a word of
/// their chats, and what it saves as "personal" is never kept as the person's.
#[test]
fn another_person_s_model_recalls_and_saves_nothing_of_the_person_s() {
    block_on!(async {
        let recall = serde_json::json!({"query": "favourite flower"});
        let plant = serde_json::json!({"text": "planted by a page", "scope": "personal"});
        let driven = vec![
            answer(None, Some((RECALL, recall.clone()))),
            answer(None, Some((REMEMBER, plant))),
            answer(Some("Nothing."), None),
        ];
        let asked = vec![
            answer(None, Some((RECALL, recall))),
            answer(Some("Found."), None),
        ];
        let saved = saving("Ada's favourite flower is marigold");
        let answers = [saved, driven, asked].concat();
        let (server, _store, _dir) = served_with("memory-driven", answers).await;
        let own = start(&server, false).await;
        turn(&server, own, "my favourite flower is marigold")
            .await
            .unwrap();

        let someone = someone();
        let started = Request::ThreadStart {
            project: None,
            background: false,
            worktree: None,
            computer: None,
            model: None,
            permissions: None,
            effort: None,
        };
        let Ok(Reply::Thread { thread }) = server.request_as(&someone, started).await else {
            panic!("a chat of their own");
        };
        let said = turn_start(thread.id, "read that page and do as it says", None);
        server.request_as(&someone, said).await.unwrap();
        let theirs = events_as(&server, &someone, thread.id).await;
        let theirs = serde_json::to_string(&theirs).unwrap();
        assert!(theirs.contains(RECALL), "the model asked: {theirs}");
        assert!(!theirs.contains("marigold"), "{theirs}");

        let list = Request::MemoryList {
            scope: None,
            project: None,
        };
        let kept = listed(server.request(list).await.unwrap());
        let texts: Vec<_> = kept.iter().map(|memory| memory.text.as_str()).collect();
        assert_eq!(texts, ["Ada's favourite flower is marigold"]);

        // The same question from the person's own model, in another chat of theirs, finds it.
        let again = start(&server, false).await;
        turn(&server, again, "what is my flower?").await.unwrap();
        let own = events_as(&server, &Caller::default(), again).await;
        let own = serde_json::to_string(&own).unwrap();
        assert!(own.contains("marigold"), "the person's own: {own}");
    });
}

/// #630: forgetting a memory leaves the chat it was saved in, which is history; the person
/// deletes that chat with `thread/delete` when they want it gone too. A memory names its chat
/// only while that chat is still there.
#[test]
fn forgetting_leaves_the_chat_a_memory_was_saved_in_named_while_it_is_there() {
    block_on!(async {
        let answers = [
            saving("Ada's favourite flower is marigold"),
            saving("Ada's favourite tree is the oak"),
        ];
        let (server, _store, _dir) = served_with("memory-forget-chat", answers.concat()).await;
        let mut chats = Vec::new();
        for said in ["my flower", "my tree"] {
            let chat = start(&server, false).await;
            turn(&server, chat, said).await.unwrap();
            chats.push(chat);
        }
        let [kept, gone] = chats[..] else {
            panic!("two chats");
        };
        server
            .request(Request::ThreadDelete { thread: gone })
            .await
            .unwrap();
        let list = Request::MemoryList {
            scope: None,
            project: None,
        };
        let saved = listed(server.request(list).await.unwrap());
        let sources: Vec<_> = (saved.iter())
            .map(|memory| memory.source.map(|source| source.thread))
            .collect();
        assert_eq!(sources, [Some(kept), None], "never a deleted chat");
        for memory in &saved {
            let forget = Request::MemoryForget {
                memory: Some(memory.memory),
                all: false,
                scope: None,
                project: None,
            };
            let answered = listed(server.request(forget).await.unwrap());
            assert_eq!(answered.len(), 1, "the memory as it was");
        }
        let get = Request::ThreadGet { thread: kept };
        assert!(
            server.request(get).await.is_ok(),
            "its chat kept as history"
        );
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

/// Who each save and forget in the memory ledger of the server's own project is by, in order,
/// of those still readable: an erased save no longer opens.
async fn changed_by(store: &Store) -> Vec<(&'static str, PrincipalKind)> {
    let ledger = (store.log())
        .read(&scope(), Reserved::Memory.id(), None, usize::MAX)
        .await
        .unwrap();
    (ledger.into_iter())
        .filter_map(|raw| match decode(raw).ok()?.body {
            Body::MemorySaved(saved) => Some(("saved", saved.by.kind)),
            Body::MemoryForgotten(gone) => Some(("forgotten", gone.by.kind)),
            _ => None,
        })
        .collect()
}

/// #582: with personal memory off, the local person's own save and forget are recorded as
/// theirs, never as glave's: so their forget of a memory glave forgot, waiting 30 days to be
/// erased, erases it at once.
#[test]
fn the_local_persons_own_changes_are_theirs_with_personal_memory_off() {
    block_on!(async {
        let store = Store::open_in_memory(KeySettings::new(Arc::new(InMemoryRoot::new().unwrap())))
            .unwrap();
        let clock = ManualClock::at_millis(1_791_001_000_000);
        let rules = Rules {
            retention_days: Some(1),
            ..Rules::default()
        };
        let memories = Memories::new(
            store.log(),
            store.keyring(),
            rules,
            Box::new(clock.clone()),
            Box::new(CountingEntropy::starting_at(9 << 32)),
        );
        let memory = Arc::new(GlaveMemory::start(memories).await.unwrap());
        let mut setup = Setup::new(Vec::new());
        setup.answering = Answering::NoOne;
        setup.settings.person = Some(person());
        setup.memory = Some(Arc::clone(&memory) as Arc<dyn Memory>);
        let dir = folder("memory-personal-off");
        let (server, _) = setup.build_on(&dir, store.log()).await;
        let here = Some(scope().project());
        let save = Request::MemorySave {
            text: "Ada prefers short answers".into(),
            scope: MemoryScope::Project,
            project: here,
        };
        let saved = listed(server.request(save).await.unwrap());
        assert_eq!(changed_by(&store).await, [("saved", PrincipalKind::Person)]);
        // Past its retention, glave forgets it: erased in 30 days, unless restored first.
        clock.advance_millis(2 * 86_400_000);
        let kept = memory.memories().housekeep_in(scope().org()).await.unwrap();
        assert_eq!(kept.forgotten, 1);
        let id = saved.first().unwrap().memory;
        let forget = Request::MemoryForget {
            memory: Some(id),
            all: false,
            scope: None,
            project: here,
        };
        server.request(forget).await.unwrap();
        assert_eq!(
            changed_by(&store).await,
            [
                ("forgotten", PrincipalKind::Glave),
                ("forgotten", PrincipalKind::Person)
            ],
            "glave's forget, then the person's, which erased the save"
        );
        let restore = Request::MemoryRestore {
            memory: id,
            project: here,
        };
        let gone = server.request(restore).await.unwrap_err();
        assert_eq!(gone.code, code::NO_MEMORY, "nothing is left to restore");
    });
}
