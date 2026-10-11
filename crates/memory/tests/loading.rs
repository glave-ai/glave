//! Memory as the model sees it, end to end through the harness and glave's MCP memory server
//! on a sealed store: the start after AGENTS.md, never rewritten; a note when a thread is
//! returned to after its memories changed; the same start and notes when the thread is resumed;
//! the start rebuilt by compaction; and nothing at all where memory is off.

#[cfg(test)]
#[path = "support/kit.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod loading {
    use glave_event::{MemoryScope, Principal};
    use glave_harness::{AllowAll, Cancel, REMEMBER};
    use glave_memory::{Remembering, Writer};
    use glave_ports::Message;
    use serde_json::json;

    use super::support::*;

    fn writer() -> Writer {
        Writer {
            person: Some(person()),
            scope: shared(),
            by: Principal::person(person()),
            source: None,
            untrusted_read: false,
        }
    }

    async fn seed(kit: &Kit, text: &str, which: MemoryScope) -> glave_event::MemoryId {
        let memories = kit.memories(rules(), 90);
        match memories
            .remember(&writer(), text, which, None, true)
            .await
            .unwrap()
        {
            Remembering::Saved { memory, .. } => memory.memory,
            Remembering::Similar(_) => unreachable!("forced"),
        }
    }

    fn first(seen: &Seen, request: usize) -> Message {
        seen.lock().unwrap()[request].messages[0].clone()
    }

    #[test]
    fn the_start_follows_agents_md_and_never_changes_across_turns() {
        block_on(async {
            let kit = Kit::new();
            seed(&kit, "Ada prefers short answers", MemoryScope::Personal).await;
            seed(&kit, "Tests need make db first", MemoryScope::Space).await;
            let memory = kit.memory(rules(), 1).await;
            let dir = folder("start");
            let answers = vec![said("hi"), said("ok")];
            let (harness, seen) = kit
                .harness(&dir, answers, AllowAll, Some(memory), settings())
                .await;
            let mut thread = harness.start_thread(shared()).await.unwrap();
            let mut quiet = |_| {};
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut quiet)
                .await
                .unwrap();
            harness
                .run_turn(&mut thread, "again", &Cancel::new(), &mut quiet)
                .await
                .unwrap();
            let texts = user_texts(&first(&seen, 0));
            assert_eq!(texts.len(), 3, "{texts:?}");
            assert!(texts[0].starts_with("# AGENTS.md"), "{texts:?}");
            assert!(texts[1].starts_with("# Memory"), "{texts:?}");
            let personal = texts[1].find("Ada prefers short answers").unwrap();
            let space = texts[1].find("Tests need make db first").unwrap();
            assert!(personal < space, "{}", texts[1]);
            assert!(
                texts[1].contains("(personal, saved 2026-10-03)"),
                "{}",
                texts[1]
            );
            assert_eq!(texts[2], "hello");
            assert_eq!(
                first(&seen, 1),
                first(&seen, 0),
                "the start is never rewritten"
            );
            let tools: Vec<String> = seen.lock().unwrap()[0]
                .tools
                .iter()
                .map(|t| t.name.clone())
                .collect();
            assert!(tools.contains(&REMEMBER.to_owned()), "{tools:?}");
            let stored = format!("{:?}", thread.messages());
            assert!(
                !stored.contains("Ada prefers"),
                "the thread never holds a memory's text"
            );
        });
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "one scenario, end to end")]
    fn returning_after_changes_gets_a_note_and_resuming_rebuilds_the_same_requests() {
        block_on(async {
            let kit = Kit::new();
            let kept = seed(&kit, "Tests need make db first", MemoryScope::Space).await;
            let memory = kit.memory(rules(), 1).await;
            let dir = folder("note");
            let save = json!({"text": "Use tabs in Python", "scope": "space"});
            let answers = vec![
                answer(None, vec![(REMEMBER, save)]),
                said("saved"),
                said("noted"),
            ];
            let (harness, seen) = kit
                .harness(&dir, answers, AllowAll, Some(memory.clone()), settings())
                .await;
            let mut thread = harness.start_thread(shared()).await.unwrap();
            let mut shown = Vec::new();
            let mut live = |update| shown.push(format!("{update:?}"));
            harness
                .run_turn(&mut thread, "remember tabs", &Cancel::new(), &mut live)
                .await
                .unwrap();
            assert!(shown.iter().any(|s| s.contains("Remembered")), "{shown:?}");
            // Meanwhile, from another thread or client:
            let elsewhere = seed(&kit, "We deploy with Render", MemoryScope::Space).await;
            let memories = kit.memories(rules(), 91);
            memories.forget(&writer(), kept).await.unwrap();
            harness
                .run_turn(&mut thread, "go on", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let requests = seen.lock().unwrap().clone();
            let last = requests.last().unwrap();
            let returning = user_texts(last.messages.last().unwrap());
            assert_eq!(returning.len(), 2, "{returning:?}");
            let note = &returning[0];
            assert!(
                note.starts_with("[Memory updates since we last spoke]"),
                "{note}"
            );
            assert!(note.contains(&format!("added [{elsewhere}]")), "{note}");
            assert!(note.contains(&format!("forgotten [{kept}]")), "{note}");
            assert!(
                !note.contains("Use tabs"),
                "the thread's own save needs no note: {note}"
            );
            assert_eq!(returning[1], "go on");
            assert_eq!(
                last.messages[0], requests[0].messages[0],
                "the start stays as it was"
            );

            // Another process resumes the thread: the start and the note read as before, except
            // the forgotten memory, which can no longer be opened.
            let resumed_memory = kit.memory(rules(), 2).await;
            let (again, seen_again) = kit
                .harness(
                    &dir,
                    vec![said("back")],
                    AllowAll,
                    Some(resumed_memory),
                    settings(),
                )
                .await;
            let mut resumed = again.resume_thread(shared(), thread.id).await.unwrap();
            again
                .run_turn(&mut resumed, "once more", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let rebuilt = seen_again.lock().unwrap()[0].clone();
            let start = user_texts(&rebuilt.messages[0]);
            assert!(
                !start.iter().any(|t| t.contains("make db")),
                "forgotten: {start:?}"
            );
            let same_note = rebuilt
                .messages
                .iter()
                .find(|m| {
                    user_texts(m)
                        .iter()
                        .any(|t| t.starts_with("[Memory updates"))
                })
                .map(user_texts)
                .unwrap();
            assert_eq!(same_note[0], *note, "the note reads as it did");
            assert!(
                !user_texts(rebuilt.messages.last().unwrap())[0].starts_with("[Memory updates"),
                "nothing changed since: no new note"
            );
        });
    }

    #[test]
    fn compaction_rebuilds_the_start_with_everything_known_and_folds_the_notes() {
        block_on(async {
            let kit = Kit::new();
            seed(&kit, "Tests need make db first", MemoryScope::Space).await;
            let memory = kit.memory(rules(), 1).await;
            let dir = folder("compact");
            let answers = vec![
                said("one"),
                said("two"),
                said("Summary: talked."),
                said("three"),
            ];
            let (harness, seen) = kit
                .harness(&dir, answers, AllowAll, Some(memory), settings())
                .await;
            let mut thread = harness.start_thread(shared()).await.unwrap();
            harness
                .run_turn(&mut thread, "first", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            seed(&kit, "We deploy with Render", MemoryScope::Space).await;
            harness
                .run_turn(&mut thread, "second", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            harness
                .compact(&mut thread, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let summary_request = seen.lock().unwrap()[2].clone();
            assert!(
                !format!("{:?}", summary_request.messages).contains("# Memory"),
                "the summary is asked for without memory, so it never stores a memory's text"
            );
            harness
                .run_turn(&mut thread, "third", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let after = seen.lock().unwrap()[3].clone();
            let start = user_texts(&after.messages[0]);
            let block = start.iter().find(|t| t.starts_with("# Memory")).unwrap();
            assert!(
                block.contains("make db") && block.contains("Render"),
                "{block}"
            );
            assert!(
                !format!("{:?}", after.messages).contains("[Memory updates"),
                "notes fold away"
            );
        });
    }

    #[test]
    fn where_memory_is_off_the_model_is_sent_exactly_what_it_is_sent_without_memory() {
        block_on(async {
            let dir = folder("off");
            let script = || vec![said("hi"), said("ok")];
            // Each run on a store of its own, with clocks that read the same.
            let run = |with_memory: bool| {
                let dir = &dir;
                async move {
                    let kit = Kit::new();
                    let memory = if with_memory {
                        Some(kit.memory(rules(), 1).await)
                    } else {
                        None
                    };
                    let (harness, seen) = kit
                        .harness(dir, script(), AllowAll, memory, settings())
                        .await;
                    let mut thread = harness.start_thread(off()).await.unwrap();
                    harness
                        .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                        .await
                        .unwrap();
                    harness
                        .run_turn(&mut thread, "again", &Cancel::new(), &mut |_| {})
                        .await
                        .unwrap();
                    seen.lock().unwrap().clone()
                }
            };
            let without = run(false).await;
            let with = run(true).await;
            assert_eq!(format!("{without:?}"), format!("{with:?}"));
            assert!(
                !format!("{with:?}").contains(glave_harness::RECALL),
                "no memory tool, recall included, is offered where memory is off"
            );
        });
    }

    #[test]
    fn the_model_cannot_save_personal_memory_from_an_isolated_space() {
        block_on(async {
            let kit = Kit::new();
            let memory = kit.memory(rules(), 1).await;
            let dir = folder("isolated");
            let save = json!({"text": "The client is BLUEHERON", "scope": "personal"});
            let answers = vec![answer(None, vec![(REMEMBER, save)]), said("done")];
            let (harness, seen) = kit
                .harness(&dir, answers, AllowAll, Some(memory), settings())
                .await;
            let mut thread = harness.start_thread(isolated()).await.unwrap();
            harness
                .run_turn(&mut thread, "remember it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let last = seen.lock().unwrap().last().unwrap().clone();
            let result = format!("{:?}", last.messages.last().unwrap());
            assert!(result.contains("isolated"), "{result}");
            let memories = kit.memories(rules(), 9);
            let everywhere = [inbox(), isolated(), shared()];
            for scope in everywhere {
                let mut on_scope = writer();
                on_scope.scope = scope;
                assert!(
                    memories
                        .list_as(Some(person()), scope, None)
                        .await
                        .unwrap()
                        .is_empty(),
                    "nothing was saved in {scope}"
                );
            }
        });
    }

    #[test]
    fn a_save_with_every_argument_filled_in_and_no_memory_to_replace_is_saved() {
        block_on(async {
            let kit = Kit::new();
            let memory = kit.memory(rules(), 1).await;
            let dir = folder("filled");
            let empty = json!({"text": "Use tabs in Python", "scope": "space", "replaces": "", "force": false});
            let none = json!({"text": "We deploy with Render", "scope": "space", "replaces": "none", "force": false});
            let junk = json!({"text": "Tests need make db", "scope": "space", "replaces": "new", "force": false});
            let answers = vec![
                answer(
                    None,
                    vec![(REMEMBER, empty), (REMEMBER, none), (REMEMBER, junk)],
                ),
                said("done"),
            ];
            let (harness, seen) = kit
                .harness(&dir, answers, AllowAll, Some(memory), settings())
                .await;
            let mut thread = harness.start_thread(shared()).await.unwrap();
            harness
                .run_turn(&mut thread, "remember these", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let saved: Vec<String> = kit
                .memories(rules(), 9)
                .list_as(Some(person()), shared(), None)
                .await
                .unwrap()
                .into_iter()
                .map(|memory| memory.text)
                .collect();
            assert_eq!(saved, ["Use tabs in Python", "We deploy with Render"]);
            let last = seen.lock().unwrap().last().unwrap().clone();
            let results = format!("{:?}", last.messages);
            assert!(
                results.contains("\\\"new\\\" is not a memory id"),
                "{results}"
            );
        });
    }
}
