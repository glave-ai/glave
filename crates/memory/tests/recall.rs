//! `recall` on a sealed store: memories and past conversations found by keyword, scoped
//! exactly as memory is (the person's and this space's in a shared space, this space's only in
//! an isolated one, never another organization's), labelled with their source, the asking
//! thread left out, untrusted output marked so and counted as untrusted content read.

#[cfg(test)]
#[path = "support/kit.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod recall {
    use glave_event::{
        At, CallId, EventId, Header, MemoryScope, MessageAdded, Part, Principal, RawEvent, Role,
        Seq, ThreadId, ToolCallCompleted, encode,
    };
    use glave_harness::{AllowAll, Cancel, MemoryContext, RECALL};
    use glave_memory::{MOST_HITS, Recalled, Source, Writer};
    use glave_ports::{EventLog, Scope};
    use serde_json::json;

    use super::support::*;

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
                MemoryScope::Space,
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
    fn a_shared_space_finds_memories_its_threads_and_the_persons_never_another_space() {
        block_on(async {
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
                "never the isolated space's, nor another organization's"
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
                "an isolated space reads only itself"
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
    fn the_model_recalls_through_the_server_and_untrusted_hits_mark_the_thread() {
        block_on(async {
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
}
