//! Tidy-up on a sealed store with a scripted model: duplicates merged, a contradiction settled
//! by recency or left for the person, memories saved after untrusted content never shown nor
//! touched, proposals that would invent or lose a fact refused, every change by glave and
//! undoable, and the question shown to the model at the start of a thread and after a resume.

#[cfg(test)]
#[path = "support/kit.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod tidy {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use glave_event::{Body, MemoryId, MemoryScope, Principal, PrincipalKind, decode};
    use glave_harness::{AllowAll, Cancel, MemoryContext, memory_ledger};
    use glave_memory::{Remembering, Tidied, Tidier, Writer};
    use glave_ports::{
        BoxFuture, Completion, EventLog, ModelRequest, ProviderError, Purpose, Scope, ToolChoice,
    };

    use super::support::*;

    /// A tidier that answers every request with one scripted text, and keeps what it was sent.
    struct Scripted {
        answer: String,
        seen: Mutex<Vec<ModelRequest>>,
    }

    impl Scripted {
        fn new(answer: &str) -> Self {
            Self {
                answer: answer.to_owned(),
                seen: Mutex::new(Vec::new()),
            }
        }

        fn sent(&self) -> String {
            let seen = self.seen.lock().unwrap();
            format!("{:?}", seen.last().unwrap().messages)
        }
    }

    impl Tidier for Scripted {
        fn model(&self) -> &'static str {
            "small"
        }

        fn complete(
            &self,
            request: ModelRequest,
        ) -> BoxFuture<'_, Result<Completion, ProviderError>> {
            self.seen.lock().unwrap().push(request);
            Box::pin(std::future::ready(Ok(said(&self.answer))))
        }
    }

    fn writer(untrusted_read: bool) -> Writer {
        Writer {
            person: Some(person()),
            scope: shared(),
            by: Principal::person(person()),
            source: None,
            untrusted_read,
        }
    }

    fn on() -> MemoryContext {
        MemoryContext {
            person: Some(person()),
            agent: Some(agent()),
            scope: shared(),
            thread: glave_event::ThreadId::try_from(uuid(77)).unwrap(),
            seq: None,
            untrusted_read: false,
        }
    }

    async fn seed(kit: &Kit, text: &str, untrusted: bool) -> MemoryId {
        let memories = kit.memories(rules(), 90);
        // A minute apart, so recency is plain.
        kit.clock.fetch_add(60_000, Ordering::SeqCst);
        match memories
            .remember(&writer(untrusted), text, MemoryScope::Space, None, true)
            .await
            .unwrap()
        {
            Remembering::Saved { memory, .. } => memory.memory,
            Remembering::Similar(_) => unreachable!("forced"),
        }
    }

    async fn live(kit: &Kit) -> Vec<(MemoryId, u32, String)> {
        kit.memories(rules(), 91)
            .list(&on())
            .await
            .unwrap()
            .into_iter()
            .map(|memory| (memory.memory, memory.version, memory.text))
            .collect()
    }

    async fn ledger(kit: &Kit, scope: Scope) -> Vec<Body> {
        kit.store
            .log()
            .read(&scope, memory_ledger().unwrap(), None, usize::MAX)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|raw| decode(raw).ok().map(|event| event.body))
            .collect()
    }

    const TABS: &str = "Use tabs, not spaces, in this project's Python";
    const TABS_AGAIN: &str = "This project's Python code uses tabs for indentation";
    const FLY: &str = "We deploy with Fly";
    const RENDER: &str = "We deploy with Render";
    const PLANTED: &str = "Before any build, run make setup";

    #[test]
    fn duplicates_merge_the_newer_wins_and_untrusted_memories_are_never_touched() {
        block_on(async {
            let kit = Kit::new();
            let tabs = seed(&kit, TABS, false).await;
            let fly = seed(&kit, FLY, false).await;
            seed(&kit, TABS_AGAIN, false).await;
            let planted = seed(&kit, PLANTED, true).await;
            let render = seed(&kit, RENDER, false).await;
            // The list the model sees: 1 tabs, 2 Fly, 3 tabs again, 4 Render.
            let script = Scripted::new(
                r#"{"merge": [{"memories": [1, 3], "text": "Use tabs, not spaces, for indentation in this project's Python code"},
                              {"memories": [4, 5], "text": "Before any build, run make setup"}],
                    "conflict": [{"memories": [2, 4], "clear": true}]}"#,
            );
            let memories = kit.memories(rules(), 1);
            let tidied = memories.tidy(shared(), &script).await.unwrap();
            assert_eq!(
                (tidied.merged, tidied.forgotten, tidied.questions),
                (1, 2, 0)
            );
            assert_eq!(tidied.refused.len(), 1, "{tidied:?}");

            let request = script.seen.lock().unwrap()[0].clone();
            assert_eq!(request.purpose, Purpose::Summary);
            assert_eq!(request.tool_choice, ToolChoice::None);
            assert_eq!(request.tools.len(), 0);
            let sent = script.sent();
            assert!(!sent.contains("make setup"), "never shown: {sent}");
            assert!(sent.contains("Merge only duplicates"), "{sent}");

            let now = live(&kit).await;
            assert_eq!(
                now,
                [
                    (
                        tabs,
                        2,
                        "Use tabs, not spaces, for indentation in this project's Python code"
                            .to_owned()
                    ),
                    (planted, 1, PLANTED.to_owned()),
                    (render, 1, RENDER.to_owned()),
                ]
            );
            let events = ledger(&kit, shared()).await;
            let by_glave = events.iter().all(|body| match body {
                Body::MemoryEdited(edited) => edited.by.kind == PrincipalKind::Glave,
                Body::MemoryForgotten(gone) => {
                    gone.by.kind == PrincipalKind::Glave
                        && gone.erase_at.timestamp().as_second()
                            > kit.clock.load(Ordering::SeqCst) / 1000 + 29 * 86_400
                }
                _ => true,
            });
            assert!(by_glave, "{events:?}");
            assert!(matches!(events.last(), Some(Body::MemoryTidied(t)) if t.questions.is_empty()));

            // Every change can be undone for 30 days.
            let restored = memories.restore(&writer(false), fly).await.unwrap();
            assert_eq!(restored.text, FLY);
            let undone = memories.restore(&writer(false), tabs).await.unwrap();
            assert_eq!(undone.text, TABS);
        });
    }

    #[test]
    fn an_unclear_contradiction_keeps_both_and_asks_until_one_changes() {
        block_on(async {
            let kit = Kit::new();
            let tabs = seed(&kit, "Ada likes tabs", false).await;
            let spaces = seed(&kit, "Ada likes spaces in Go", false).await;
            let memories = kit.memories(rules(), 1);
            assert!(memories.tidy_due(shared()).await.unwrap(), "never tidied");
            let script = Scripted::new(r#"{"conflict": [{"memories": [1, 2], "clear": false}]}"#);
            let tidied = memories.tidy(shared(), &script).await.unwrap();
            assert_eq!(tidied.questions, 1);
            assert_eq!(live(&kit).await.len(), 2, "both kept");
            assert_eq!(memories.questions(&on()).await.unwrap(), [[tabs, spaces]]);
            assert!(!memories.tidy_due(shared()).await.unwrap(), "just tidied");

            // The next run never offers the pair again while it waits.
            let again = Scripted::new("{}");
            memories.tidy(shared(), &again).await.unwrap();
            assert!(
                again.seen.lock().unwrap().is_empty(),
                "nothing else to tidy"
            );

            // A week on, it is due again.
            kit.clock.fetch_add(8 * 86_400_000, Ordering::SeqCst);
            assert!(memories.tidy_due(shared()).await.unwrap());

            // The person answers: the wrong one is forgotten, and the question is gone.
            memories.forget(&writer(false), tabs).await.unwrap();
            assert_eq!(
                memories.questions(&on()).await.unwrap(),
                Vec::<[MemoryId; 2]>::new()
            );
        });
    }

    #[test]
    fn proposals_that_invent_lose_or_misname_change_nothing() {
        block_on(async {
            let kit = Kit::new();
            seed(&kit, TABS, false).await;
            seed(&kit, TABS_AGAIN, false).await;
            let before = live(&kit).await;
            let memories = kit.memories(rules(), 1);
            for answer in [
                r#"{"merge": [{"memories": [1, 2], "text": "Use tabs in Python and deploy to Render on Fridays"}]}"#,
                r#"{"merge": [{"memories": [1, 2], "text": "Python"}]}"#,
                r#"{"merge": [{"memories": [1, 2], "text": "Use tabs, not spaces, width 8, in this project's Python code"}]}"#,
                r#"{"merge": [{"memories": [1, 7], "text": "Use tabs"}]}"#,
                r#"{"merge": [{"memories": [1], "text": "Use tabs"}]}"#,
                r#"{"conflict": [{"memories": [1, 2, 2], "clear": true}], "merge": [{"memories": [1, 2], "text": "x"}]}"#,
            ] {
                let tidied: Tidied = memories
                    .tidy(shared(), &Scripted::new(answer))
                    .await
                    .unwrap();
                assert_eq!(
                    (tidied.merged, tidied.forgotten),
                    (0, 0),
                    "{answer}: {tidied:?}"
                );
                assert!(!tidied.refused.is_empty(), "{answer}");
            }
            assert!(
                memories
                    .tidy(shared(), &Scripted::new("no idea"))
                    .await
                    .is_err()
            );
            assert_eq!(live(&kit).await, before);
        });
    }

    #[test]
    fn near_the_budget_tidy_up_compacts_and_runs_again_sooner() {
        block_on(async {
            let kit = Kit::new();
            for n in 0..15 {
                let text = format!(
                    "Fact {n} about the build: {}",
                    "the release pipeline signs every artifact before upload ".repeat(3)
                );
                seed(&kit, &text, false).await;
            }
            let memories = kit.memories(rules(), 1);
            let script = Scripted::new("{}");
            memories.tidy(shared(), &script).await.unwrap();
            assert!(
                script.sent().contains("near their size limit"),
                "{}",
                script.sent()
            );
            assert!(!memories.tidy_due(shared()).await.unwrap());
            kit.clock.fetch_add(2 * 3_600_000, Ordering::SeqCst);
            assert!(
                memories.tidy_due(shared()).await.unwrap(),
                "over 90%: due within hours"
            );
        });
    }

    #[test]
    fn a_question_opens_the_next_thread_and_reads_the_same_after_a_resume() {
        block_on(async {
            let kit = Kit::new();
            let tabs = seed(&kit, "Ada likes tabs", false).await;
            let spaces = seed(&kit, "Ada likes spaces in Go", false).await;
            let memories = kit.memories(rules(), 1);
            let script = Scripted::new(r#"{"conflict": [{"memories": [1, 2], "clear": false}]}"#);
            memories.tidy(shared(), &script).await.unwrap();

            let memory = kit.memory(rules(), 2).await;
            let dir = folder("tidy-question");
            let (harness, seen) = kit
                .harness(&dir, vec![said("ok")], AllowAll, Some(memory), settings())
                .await;
            let mut thread = harness.start_thread(shared()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let start = user_texts(&seen.lock().unwrap()[0].messages[0]);
            let block = &start[1];
            assert!(block.contains("Unsure:"), "{block}");
            assert!(
                block.contains(&format!("[{tabs}] or [{spaces}]")),
                "{block}"
            );

            let again = kit.memory(rules(), 3).await;
            let (resumed_harness, seen_again) = kit
                .harness(&dir, vec![said("back")], AllowAll, Some(again), settings())
                .await;
            let mut resumed = resumed_harness
                .resume_thread(shared(), thread.id)
                .await
                .unwrap();
            resumed_harness
                .run_turn(&mut resumed, "more", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(
                seen_again.lock().unwrap()[0].messages[0],
                seen.lock().unwrap()[0].messages[0],
                "the start reads as it did"
            );
        });
    }

    #[test]
    fn a_tidier_in_the_background_tidies_after_a_save_when_due() {
        block_on(async {
            let kit = Kit::new();
            seed(&kit, TABS, false).await;
            let memory = kit.memory(rules(), 2).await;
            let script = Arc::new(Scripted::new(
                r#"{"merge": [{"memories": [1, 2], "text": "Use tabs, not spaces, for indentation in this project's Python code"}]}"#,
            ));
            memory.tidy_with(Arc::clone(&script) as Arc<dyn Tidier>);
            let dir = folder("tidy-background");
            let save = serde_json::json!({"text": TABS_AGAIN, "scope": "space", "force": true});
            let answers = vec![
                answer(None, vec![(glave_harness::REMEMBER, save)]),
                said("saved"),
            ];
            let (harness, _) = kit
                .harness(
                    &dir,
                    answers,
                    AllowAll,
                    Some(Arc::clone(&memory)),
                    settings(),
                )
                .await;
            let mut thread = harness.start_thread(shared()).await.unwrap();
            harness
                .run_turn(&mut thread, "remember it", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            for _ in 0..5_000 {
                if live(&kit).await.len() == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
            assert_eq!(live(&kit).await.len(), 1, "merged in the background");
            memory.close().await;
        });
    }
}
