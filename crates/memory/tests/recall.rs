//! `recall` on a sealed store: memories and past conversations found by keyword, scoped
//! exactly as memory is (the person's and this project's in a shared project, this project's only in
//! an isolated one, never another organization's), labelled with their source, the asking
//! thread left out, untrusted output marked so and counted as untrusted content read.

use glave_event::{
    Arguments, At, CallId, EventId, Header, MemoryScope, MessageAdded, Part, Principal, RawEvent,
    Role, Seq, ThreadId, ToolCallCompleted, encode,
};
use glave_harness::{AllowAll, Cancel};
use glave_memory::{MOST_HITS, Recalled, Source, Writer};
use glave_ports::memories::{Memory, MemoryContext, RECALL, REMEMBER};
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

/// #578: a memory's text that came back to the model through a tool, here `recall`'s result,
/// stays nowhere a forget cannot reach: not in the thread's log, so not in the search index
/// either, and a thread read again after the forget no longer shows it to the model.
#[test]
#[expect(clippy::too_many_lines, reason = "one scenario, end to end")]
fn a_forgotten_memory_is_never_found_again_through_an_earlier_recall() {
    block_on!(async {
        let kit = Kit::new();
        let memory = kit.memory(rules(), 2).await;
        let dir = folder("recall-copies");
        let save = json!({"text": "The favourite flower is marigold", "scope": "personal"});
        let answers = vec![
            answer(None, vec![(REMEMBER, save)]),
            said("Saved."),
            answer(None, vec![(RECALL, json!({"query": "favourite flower"}))]),
            said("Found it."),
        ];
        let (harness, seen) = kit
            .harness(&dir, answers, AllowAll, Some(memory), settings())
            .await;
        let mut saving = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(
                &mut saving,
                "Remember: my favourite flower is marigold.",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let saved = kit.results(shared(), saving.id).await;
        assert!(
            saved.len() == 1 && saved.iter().all(|kept| !kept.contains("marigold")),
            "the save's own result is kept without the text"
        );
        let mut recalling = harness.start_thread(shared()).await.unwrap();
        harness
            .run_turn(
                &mut recalling,
                "Which flower do I like best?",
                &Cancel::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        let live = seen.lock().unwrap().last().unwrap().messages.clone();
        let result = format!("{:?}", live.last());
        assert!(
            result.contains("The favourite flower is marigold")
                && result.contains("Remember: my favourite flower is marigold."),
            "the model reads what recall found"
        );
        // Read again by another process, the thread reads as it did.
        let (again, seen_again) = kit
            .harness(
                &dir,
                vec![said("Yes.")],
                AllowAll,
                Some(kit.memory(rules(), 6).await),
                settings(),
            )
            .await;
        let mut resumed = again.resume_thread(shared(), recalling.id).await.unwrap();
        again
            .run_turn(&mut resumed, "Still sure?", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let rebuilt = seen_again.lock().unwrap()[0].messages.clone();
        assert!(
            rebuilt.get(..live.len()) == Some(live.as_slice()),
            "resumed, the model reads what it read"
        );

        let memories = kit.memories(rules(), 3);
        let saved = memories
            .list_as(Some(person()), shared(), None)
            .await
            .unwrap()
            .first()
            .map(|memory| memory.memory)
            .unwrap();
        let writer = Writer {
            person: Some(person()),
            scope: shared(),
            by: Principal::person(person()),
            source: None,
            untrusted_read: false,
        };
        memories.forget(&writer, saved).await.unwrap();

        let fresh = kit.memories(rules(), 4);
        let after = fresh
            .recall(&on(shared(), thread(9)), "favourite flower marigold", 10)
            .await
            .unwrap();
        assert!(
            after.iter().all(|hit| !hit.text.contains("marigold")),
            "a later recall finds nothing of the forgotten memory's text: {} hits hold it",
            after
                .iter()
                .filter(|hit| hit.text.contains("marigold"))
                .count()
        );
        let kept = kit
            .store
            .log()
            .read(&shared(), recalling.id, None, usize::MAX)
            .await
            .unwrap();
        assert!(
            kept.iter()
                .all(|event| !event.data.get().contains("marigold")),
            "the recalling thread's log keeps no copy of it"
        );

        // Read again after the forget, the recalling thread no longer shows it to the model.
        let (again, seen_again) = kit
            .harness(
                &dir,
                vec![said("back")],
                AllowAll,
                Some(kit.memory(rules(), 5).await),
                settings(),
            )
            .await;
        let mut resumed = again.resume_thread(shared(), recalling.id).await.unwrap();
        again
            .run_turn(&mut resumed, "And now?", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let sent = format!("{:?}", seen_again.lock().unwrap()[0].messages);
        assert!(
            !sent.contains("marigold"),
            "resumed, the model reads none of it"
        );
    });
}

/// A recall's result kept by reference names past conversations by place, which a later
/// process reads back from the log: read again, it shows only what the thread reading it may.
#[test]
fn a_kept_recall_reads_again_only_what_its_reader_may_read() {
    block_on!(async {
        let kit = Kit::new();
        world(&kit).await;
        let memory = kit.memory(rules(), 2).await;
        let mut query = serde_json::Map::new();
        query.insert("query".to_owned(), json!("orca"));
        let found = (memory.call(&on(shared(), thread(9)), RECALL, &Arguments::Json(query))).await;
        let shown = [found.kept.unwrap().shown];
        let read = async |reader: MemoryContext| -> String {
            let again = memory.reread(&reader, &shown).await.unwrap();
            again.into_iter().flatten().collect()
        };
        let here = read(on(shared(), thread(9))).await;
        assert!(
            ["host orca", "orca-7", "being retired", "laptop"]
                .iter()
                .all(|text| here.contains(text)),
            "where it was found, it reads as found"
        );
        let stranger = MemoryContext {
            person: Some(glave_event::PrincipalId::try_from(uuid(52)).unwrap()),
            ..on(shared(), thread(9))
        };
        let project = ["host orca", "orca-7", "being retired"];
        for (who, reader, theirs) in [
            (
                "from another project",
                on(scope(1, 5), thread(9)),
                &project[..],
            ),
            (
                "from another organization",
                on(scope(9, 6), thread(9)),
                &project[..],
            ),
            ("by someone else", stranger, &["laptop"][..]),
        ] {
            let again = read(reader).await;
            assert!(
                theirs.iter().all(|text| !again.contains(text)),
                "read again {who}, it shows only what they may read"
            );
        }
        assert!(
            !read(on(scope(9, 6), thread(9))).await.contains("laptop"),
            "nor the person's own from another organization"
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
