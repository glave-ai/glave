//! `recall` on a sealed store: memories and past conversations found by keyword, scoped
//! exactly as memory is (the person's and this project's in a shared project, this project's only in
//! an isolated one, never another organization's), labelled with their source, the asking
//! thread left out, untrusted output marked so and counted as untrusted content read.

use glave_event::{
    At, CallId, EventId, Header, MemoryId, MemoryScope, MessageAdded, Part, Principal, RawEvent,
    Role, Seq, ThreadId, ToolCallCompleted, encode,
};
use glave_harness::{AllowAll, Cancel};
use glave_memory::{MOST_HITS, Recalled, Source, Tidier, Writer};
use glave_ports::memories::{MemoryContext, RECALL, REMEMBER};
use glave_ports::{EventLog, Scope};
use serde_json::json;

use crate::support::*;

fn thread(n: u128) -> ThreadId {
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

fn on(scope: Scope, asking: ThreadId) -> MemoryContext {
    MemoryContext {
        person: Some(person()),
        agent: Some(agent()),
        scope,
        thread: asking,
        seq: None,
        untrusted_read: false,
    }
}

async fn world(kit: &Kit) {
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
fn forgetting_a_memory_takes_the_turn_it_came_from_out_of_recall() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("recall-forget");
        let save = json!({"text": "The favourite flower is marigold", "scope": "personal"});
        let answers = vec![
            answer(None, vec![(REMEMBER, save)]),
            said("OK, remembered marigold."),
            said("Marigold beds need sun."),
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
        harness
            .run_turn(
                &mut first,
                "Where do marigold beds go?",
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

        // A process that never saw the memory, reading the ledger afresh, hides it too.
        let fresh = kit.memories(rules(), 4);
        let after = fresh.recall(&asking, "marigold", 10).await.unwrap();
        assert!(
            after
                .iter()
                .all(|hit| !hit.text.contains("favourite flower is marigold")
                    && !hit.text.contains("remembered marigold")),
            "{after:?}"
        );
        assert!(
            after.iter().any(|hit| hit.text.contains("beds")),
            "a later turn still is found: {after:?}"
        );
        let kept = kit
            .store
            .log()
            .read(&shared(), first.id, None, usize::MAX)
            .await
            .unwrap();
        assert!(
            kept.iter()
                .any(|event| event.data.get().contains("favourite flower is marigold")),
            "the conversation itself stays"
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
            .unwrap()
            .data
            .get()
            .to_owned();
        assert!(completed.contains("\"untrusted\":true"), "{completed}");
        assert!(completed.contains("[memory "), "{completed}");
        assert!(completed.contains("tool output, untrusted"), "{completed}");
        assert!(
            completed.contains(&format!("[thread {} #2]", super::recall::thread(2))),
            "{completed}"
        );
        assert!(!completed.contains("BLUEHERON"));
    });
}

/// What the person said, among `hits`.
fn persons(hits: &[Recalled]) -> Vec<String> {
    let mut said: Vec<String> = hits
        .iter()
        .filter(|hit| {
            matches!(
                hit.source,
                Source::Thread {
                    said_by: "person",
                    ..
                }
            )
        })
        .map(|hit| hit.text.clone())
        .collect();
    said.sort();
    said
}

const MERGED_QUERY: &str = "atlas Railway merging";

/// Two threads, each saving a memory, then tidy-up merging the two: the memories, oldest (the
/// one that takes the merged text) first.
async fn merged(kit: &Kit) -> Vec<MemoryId> {
    let memory = kit.memory(rules(), 2).await;
    let dir = folder("recall-merged");
    let save = |text: &str| {
        let args = json!({"text": text, "scope": "project", "force": true});
        answer(None, vec![(REMEMBER, args)])
    };
    let answers = vec![
        save("Atlas deploys to Railway"),
        said("Noted."),
        save("Merging to main ships atlas to Railway"),
        said("Noted."),
    ];
    // No AGENTS.md, so the person's first message is only their words.
    let mut plain = settings();
    plain.instructions = None;
    let (harness, _) = kit
        .harness(&dir, answers, AllowAll, Some(memory), plain)
        .await;
    for message in [
        "We deploy atlas to Railway.",
        "Merging to main ships atlas.",
    ] {
        let mut thread = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(&mut thread, message, &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
    }
    let memories = kit.memories(rules(), 3);
    let saved: Vec<MemoryId> = memories
        .list(&on(shared(), thread(9)))
        .await
        .unwrap()
        .iter()
        .map(|memory| memory.memory)
        .collect();
    let tidier = Tidier {
        provider: ScriptedModel::cycling(vec![said(
            r#"{"merge": [{"memories": [1, 2], "text": "Atlas deploys to Railway: merging to main ships it"}]}"#,
        )]),
        model: "small".to_owned(),
    };
    assert_eq!(memories.tidy(shared(), &tidier).await.unwrap().merged, 1);
    let hits = memories
        .recall(&on(shared(), thread(9)), MERGED_QUERY, 10)
        .await
        .unwrap();
    assert_eq!(
        persons(&hits),
        [
            "Merging to main ships atlas.",
            "We deploy atlas to Railway."
        ],
        "the merged text lives on, so both turns stay"
    );
    saved
}

fn persons_writer() -> Writer {
    Writer {
        person: Some(person()),
        scope: shared(),
        by: Principal::person(person()),
        source: None,
        untrusted_read: false,
    }
}

/// #555: tidy-up merged away the memory a turn saved, and the turn dropped out of recall though
/// the person forgot nothing.
#[test]
fn a_merge_hides_no_turn_and_forgetting_what_it_made_hides_and_erases_what_went_in() {
    block_on!(async {
        let kit = Kit::new();
        let saved = merged(&kit).await;
        let [kept, folded] = saved.as_slice() else {
            panic!("{saved:?}");
        };
        let memories = kit.memories(rules(), 4);
        memories.forget(&persons_writer(), *kept).await.unwrap();
        let after = kit
            .memories(rules(), 5)
            .recall(&on(shared(), thread(9)), MERGED_QUERY, 10)
            .await
            .unwrap();
        assert!(after.is_empty(), "{after:?}");
        assert!(
            memories.restore(&persons_writer(), *folded).await.is_err(),
            "what went into the forgotten memory is erased with it, not waiting to be restored"
        );
    });
}

#[test]
fn once_the_merged_away_memory_is_erased_forgetting_the_merge_still_hides_its_turn() {
    block_on!(async {
        let kit = Kit::new();
        let saved = merged(&kit).await;
        let kept = saved.first().copied().unwrap();
        kit.clock.advance_millis(31 * 86_400_000);
        let memories = kit.memories(rules(), 4);
        let housekept = memories.housekeep(&[shared()]).await.unwrap();
        assert_eq!(housekept.erased, 1, "{housekept:?}");
        let fresh = kit.memories(rules(), 5);
        let asking = on(shared(), thread(9));
        let hits = fresh.recall(&asking, MERGED_QUERY, 10).await.unwrap();
        assert_eq!(persons(&hits).len(), 2, "{hits:?}");
        fresh.forget(&persons_writer(), kept).await.unwrap();
        let after = kit
            .memories(rules(), 6)
            .recall(&asking, MERGED_QUERY, 10)
            .await
            .unwrap();
        assert!(after.is_empty(), "{after:?}");
    });
}

#[test]
fn a_merge_undone_then_forgotten_leaves_what_went_in_restorable() {
    block_on!(async {
        let kit = Kit::new();
        let saved = merged(&kit).await;
        let [kept, folded] = saved.as_slice() else {
            panic!("{saved:?}");
        };
        let memories = kit.memories(rules(), 4);
        let undone = memories.restore(&persons_writer(), *kept).await.unwrap();
        assert_eq!(
            undone.text, "Atlas deploys to Railway",
            "the merge's edit undone"
        );
        memories.forget(&persons_writer(), *kept).await.unwrap();
        let hits = kit
            .memories(rules(), 5)
            .recall(&on(shared(), thread(9)), MERGED_QUERY, 10)
            .await
            .unwrap();
        let same = memories
            .recall(&on(shared(), thread(9)), MERGED_QUERY, 10)
            .await
            .unwrap();
        assert_eq!(
            hits, same,
            "a view read afresh hides what the live one does"
        );
        let back = memories.restore(&persons_writer(), *folded).await.unwrap();
        assert_eq!(back.text, "Merging to main ships atlas to Railway");
        let after = kit
            .memories(rules(), 6)
            .recall(&on(shared(), thread(9)), MERGED_QUERY, 10)
            .await
            .unwrap();
        assert_eq!(
            persons(&after),
            ["Merging to main ships atlas."],
            "restored, its turn is back; the forgotten one's stays hidden"
        );
    });
}
