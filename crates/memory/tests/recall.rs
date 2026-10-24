//! `recall` on a sealed store: memories and past conversations found by keyword, scoped
//! exactly as memory is (the person's and this project's in a shared project, this project's only in
//! an isolated one, never another organization's), labelled with their source, the asking
//! thread and deleted chats left out, untrusted output marked so and counted as untrusted
//! content read.

use std::sync::Arc;

use glave_event::{
    Arguments, At, CallId, EventId, Header, MemoryScope, MessageAdded, Part, Principal, RawEvent,
    Role, Seq, ThreadId, ToolCallCompleted, encode,
};
use glave_harness::{AllowAll, Cancel, Change, change_standing};
use glave_memory::{MOST_HITS, Recalled, Source, Writer};
use glave_ports::memories::{FORGET, Memory as _, MemoryContext, RECALL, REMEMBER};
use glave_ports::{Clock as _, EventLog, Scope};
use serde_json::json;

use crate::support::*;

pub(crate) fn thread(n: u128) -> ThreadId {
    ThreadId::try_from(uuid(1_000 + n)).unwrap()
}

fn header(thread: ThreadId, seq: u64) -> Header {
    Header {
        id: EventId::try_from(uuid(
            5_000 + u128::from(seq) + thread.as_uuid().as_u128() % 1_000 * 100,
        ))
        .unwrap(),
        thread,
        seq: Seq::new(seq).unwrap(),
        at: At::try_from("2026-10-04T10:00:00Z".to_owned()).unwrap(),
    }
}

/// A past thread in `scope`: the person says `said`, then a tool returns `output`.
async fn past(kit: &Kit, scope: Scope, id: ThreadId, said: &str, output: (&str, bool)) {
    let message = MessageAdded {
        role: Role::User,
        parts: vec![Part::Text { text: said.into() }],
        author: None,
    };
    let done = ToolCallCompleted {
        call_id: CallId::new("call_1").unwrap(),
        is_error: false,
        output: vec![Part::Text {
            text: output.0.into(),
        }],
        structured: None,
        untrusted: output.1,
    };
    let events: Vec<RawEvent> = vec![
        encode(header(id, 1), &message).unwrap(),
        encode(header(id, 2), &done).unwrap(),
    ];
    kit.store.log().append(&scope, &events).await.unwrap();
}

pub(crate) fn on(scope: Scope, asking: ThreadId) -> MemoryContext {
    MemoryContext {
        person: Some(person()),
        agent: Some(agent()),
        scope,
        thread: asking,
        seq: None,
        untrusted_read: false,
    }
}

pub(crate) async fn world(kit: &Kit) {
    let memories = kit.memories(rules(), 90);
    let writer = Writer {
        person: Some(person()),
        scope: shared(),
        by: Principal::person(person()),
        source: None,
        untrusted_read: false,
    };
    memories
        .remember(
            &writer,
            "The staging database is on host orca",
            MemoryScope::Project,
            None,
            true,
        )
        .await
        .unwrap();
    past(
        kit,
        shared(),
        thread(1),
        "Remember the staging host is orca-7",
        ("ok", false),
    )
    .await;
    past(
        kit,
        shared(),
        thread(2),
        "Read the wiki",
        ("orca hosts are being retired", true),
    )
    .await;
    past(
        kit,
        inbox(),
        thread(3),
        "My laptop is called orca too",
        ("ok", false),
    )
    .await;
    past(
        kit,
        isolated(),
        thread(4),
        "The client codename is BLUEHERON on orca",
        ("ok", false),
    )
    .await;
    past(
        kit,
        scope(2, 2),
        thread(5),
        "Another organization's orca",
        ("ok", false),
    )
    .await;
}

fn sources(hits: &[Recalled]) -> Vec<String> {
    hits.iter()
        .map(|hit| match hit.source {
            Source::Memory { .. } => "memory".to_owned(),
            Source::Thread {
                thread: t,
                seq,
                said_by,
            } => {
                let n = t.as_uuid().as_u128() & 0xffff;
                format!(
                    "{}#{} {said_by}{}",
                    n - 1_000,
                    seq.get(),
                    if hit.untrusted { " untrusted" } else { "" }
                )
            }
        })
        .collect()
}

#[test]
fn a_shared_project_finds_memories_its_threads_and_the_persons_never_another_project() {
    block_on!(async {
        let kit = Kit::new();
        world(&kit).await;
        let memories = kit.memories(rules(), 1);
        let hits = memories
            .recall(&on(shared(), thread(9)), "orca", 10)
            .await
            .unwrap();
        let mut found = sources(&hits);
        assert_eq!(
            found.first().map(String::as_str),
            Some("memory"),
            "{found:?}"
        );
        found.sort();
        assert_eq!(
            found,
            [
                "1#1 person",
                "2#2 tool output untrusted",
                "3#1 person",
                "memory"
            ],
            "never the isolated project's, nor another organization's"
        );
        assert!(hits.iter().all(|hit| !hit.text.contains("BLUEHERON")));

        let asking = memories
            .recall(&on(shared(), thread(1)), "orca", 10)
            .await
            .unwrap();
        assert!(
            !sources(&asking).iter().any(|s| s.starts_with("1#")),
            "the asking thread is left out"
        );

        let isolated_hits = memories
            .recall(&on(isolated(), thread(9)), "orca", 10)
            .await
            .unwrap();
        assert_eq!(
            sources(&isolated_hits),
            ["4#1 person"],
            "an isolated project reads only itself"
        );
        let off_hits = memories
            .recall(&on(off(), thread(9)), "orca", 10)
            .await
            .unwrap();
        assert_eq!(off_hits.len(), 0);

        assert_eq!(
            memories
                .recall(&on(shared(), thread(9)), "orca", 1)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            memories
                .recall(&on(shared(), thread(9)), "orca", 500)
                .await
                .unwrap()
                .len()
                <= MOST_HITS
        );
        assert!(
            memories
                .recall(&on(shared(), thread(9)), "the of", 5)
                .await
                .is_err()
        );
    });
}

#[test]
fn the_persons_chats_are_one_ledger_searched_once() {
    block_on!(async {
        let kit = Kit::new();
        world(&kit).await;
        let memories = kit.memories(rules(), 1);
        let hits = memories
            .recall(&on(inbox(), thread(9)), "orca", 10)
            .await
            .unwrap();
        assert_eq!(sources(&hits), ["3#1 person"]);
    });
}

/// `memory-month`'s probe 9 (#555): a project's memories all name it, and a fact said once in
/// passing, in a thread run earlier in the same store, holds every word of the query.
#[test]
fn a_message_holding_every_word_outranks_memories_sharing_only_the_projects_name() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("recall-ranked");
        let (harness, _) = kit
            .harness(
                &dir,
                vec![said("Try Tune, Dial or Prefs.")],
                AllowAll,
                Some(memory),
                settings(),
            )
            .await;
        let mut earlier = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(
                &mut earlier,
                "Can you suggest three short, friendly names for beacon's new settings screen? \
                 I'm trying them out on the staging server, gannet-7, this afternoon.",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let memories = kit.memories(rules(), 3);
        let writer = Writer {
            person: Some(person()),
            scope: shared(),
            by: Principal::person(person()),
            source: None,
            untrusted_read: false,
        };
        for fact in [
            "Beacon uses SQLite, not Postgres: kiosk data travels to head office on a USB stick",
            "Beacon releases go out every Thursday",
            "Beacon is built with Flutter",
            "Beacon crash reports land in #beacon-crashes",
            "Beacon translations live in lib/l10n and go through Weblate",
            "Beacon needs Android 10 or later",
        ] {
            memories
                .remember(&writer, fact, MemoryScope::Project, None, true)
                .await
                .unwrap();
        }
        let hits = memories
            .recall(&on(shared(), thread(9)), "Beacon staging server", 5)
            .await
            .unwrap();
        let kinds: Vec<&str> = hits
            .iter()
            .map(|hit| match hit.source {
                Source::Memory { .. } => "memory",
                Source::Thread { thread, .. } if thread == earlier.id => "earlier",
                Source::Thread { .. } => "elsewhere",
            })
            .collect();
        assert_eq!(
            kinds,
            ["earlier", "memory", "memory", "memory", "memory"],
            "{hits:?}"
        );
        assert!(
            hits.first()
                .is_some_and(|hit| hit.text.contains("gannet-7"))
        );
    });
}

/// #630: forgetting erases the memory, and the chat it was saved in stays history, found as any
/// other; once the person deletes that chat too, recall finds nothing of it.
#[test]
fn a_forgotten_memory_s_chat_is_history_until_it_is_deleted_too() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("recall-forget");
        let save = json!({"text": "The favourite flower is marigold", "scope": "personal"});
        let answers = vec![
            answer(None, vec![(REMEMBER, save)]),
            said("OK, remembered marigold."),
        ];
        let (harness, _) = kit
            .harness(&dir, answers, AllowAll, Some(memory), settings())
            .await;
        let mut first = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(
                &mut first,
                "Remember: my favourite flower is marigold.",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let memories = kit.memories(rules(), 3);
        let asking = on(shared(), thread(9));
        let before = memories.recall(&asking, "marigold", 10).await.unwrap();
        let saved = before
            .iter()
            .find_map(|hit| match hit.source {
                Source::Memory { memory, .. } => Some(memory),
                Source::Thread { .. } => None,
            })
            .unwrap();
        let writer = Writer {
            person: Some(person()),
            scope: shared(),
            by: Principal::person(person()),
            source: None,
            untrusted_read: false,
        };
        memories.forget(&writer, saved).await.unwrap();

        // A process that never saw the memory, reading the ledger afresh.
        let after = kit
            .memories(rules(), 4)
            .recall(&asking, "marigold", 10)
            .await
            .unwrap();
        assert!(
            after.iter().all(
                |hit| matches!(hit.source, Source::Thread { thread, .. } if thread == first.id)
            ),
            "the memory is gone: {after:?}"
        );
        assert!(
            after
                .iter()
                .any(|hit| hit.text.contains("my favourite flower is marigold")),
            "the chat is history: {after:?}"
        );
        let (log, now) = (kit.store.log(), kit.clock.now());
        change_standing(&log, &shared(), first.id, now, [7; 10], Change::Delete)
            .await
            .unwrap();
        let deleted = kit
            .memories(rules(), 5)
            .recall(&asking, "marigold", 10)
            .await
            .unwrap();
        assert_eq!(deleted, [], "nothing finds it once its chat is deleted");
    });
}

/// When the model forgets a memory for the person, its result says the chat the memory was
/// saved in still mentions it and can be deleted, so the model can tell them. It says nothing of
/// a chat deleted already, nor of another person's chat.
#[test]
fn the_model_s_forget_tells_of_the_person_s_own_chat_that_still_mentions_it() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("recall-forget-says");
        let save = |text: &str| {
            let args = json!({"text": text, "scope": "project", "force": true});
            vec![answer(None, vec![(REMEMBER, args)]), said("Saved.")]
        };
        let mut someone = settings();
        someone.person = Some(glave_event::PrincipalId::try_from(uuid(52)).unwrap());
        let mut chats = Vec::new();
        for (text, settings) in [
            ("The favourite flower is marigold", settings()),
            ("The favourite tree is the oak", settings()),
            ("The favourite bird is the wren", someone),
        ] {
            let (harness, _) = (kit.harness(
                &dir,
                save(text),
                AllowAll,
                Some(Arc::clone(&memory)),
                settings,
            ))
            .await;
            let mut chat = harness.start_thread(shared()).await.unwrap();
            harness
                .run_turn(&mut chat, text, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            chats.push(chat.id);
        }
        let (log, now) = (kit.store.log(), kit.clock.now());
        change_standing(&log, &shared(), chats[1], now, [7; 10], Change::Delete)
            .await
            .unwrap();
        let asking = on(shared(), thread(9));
        let saved = memory.list(&asking).await.unwrap();
        let mut said = Vec::new();
        for forgotten in &saved {
            let mut arguments = serde_json::Map::new();
            arguments.insert("memory".to_owned(), json!(forgotten.memory.to_string()));
            let arguments = Arguments::Json(arguments);
            let out = memory.call(&asking, FORGET, &arguments).await;
            said.push(format!("{:?}", out.output.parts));
        }
        let mentions = "The chat it was saved in still mentions it";
        let told: Vec<bool> = said.iter().map(|said| said.contains(mentions)).collect();
        assert_eq!(
            told,
            [true, false, false],
            "own, deleted, another's: {said:?}"
        );
    });
}

#[test]
fn the_model_recalls_through_the_server_and_untrusted_hits_mark_the_thread() {
    block_on!(async {
        let kit = Kit::new();
        world(&kit).await;
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("recall");
        let answers = vec![
            answer(None, vec![(RECALL, json!({"query": "orca retired"}))]),
            said("found it"),
        ];
        let (harness, seen) = kit
            .harness(&dir, answers, AllowAll, Some(memory), settings())
            .await;
        let mut thread = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(&mut thread, "what about orca?", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let tools: Vec<String> = seen.lock().unwrap()[0]
            .tools
            .iter()
            .map(|t| t.name.clone())
            .collect();
        assert!(tools.contains(&RECALL.to_owned()), "{tools:?}");
        let events = kit
            .store
            .log()
            .read(&shared(), thread.id, None, usize::MAX)
            .await
            .unwrap();
        let completed = events
            .iter()
            .find(|event| event.kind.as_str() == "tool.call.completed")
            .unwrap();
        let data = completed.data.get().to_owned();
        assert!(data.contains("\"untrusted\":true"), "{data}");
        assert!(data.contains("[memory "), "{data}");
        assert!(data.contains("tool output, untrusted"), "{data}");
        assert!(
            data.contains(&format!("[thread {} #2]", super::recall::thread(2))),
            "{data}"
        );
        assert!(!data.contains("BLUEHERON"));
        assert!(
            (kit.results(shared(), thread.id).await.iter())
                .all(|kept| !kept.contains("host orca") && !kept.contains("being retired")),
            "the log keeps the result without the texts it showed"
        );
        let live = seen.lock().unwrap()[1].messages.clone();
        let read = format!("{:?}", live.last());
        assert!(
            read.contains("on host orca") && read.contains("<untrusted>"),
            "the model reads them, labelled as the kept result is"
        );
        let (again, seen_again) = kit
            .harness(
                &dir,
                vec![said("again")],
                AllowAll,
                Some(kit.memory(rules(), 3).await),
                settings(),
            )
            .await;
        let mut resumed = again.resume_thread(shared(), thread.id).await.unwrap();
        again
            .run_turn(&mut resumed, "and?", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let rebuilt = seen_again.lock().unwrap()[0].messages.clone();
        assert!(
            rebuilt.get(..live.len()) == Some(live.as_slice()),
            "resumed, the model reads the result as it did, labelled the same"
        );
    });
}
