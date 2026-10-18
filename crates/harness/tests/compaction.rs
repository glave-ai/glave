//! Compaction against a scripted model: a thread that outgrows its window is summarized,
//! recorded, continued from the summary, and resumed the same way; one that fits is left alone.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod compacting {
    use glave_event::{Part, StopReason};
    use glave_harness::{AllowAll, Cancel, Compaction, Live};
    use glave_ports::{InMemoryLog, Message, ModelRequest};
    use serde_json::json;

    use super::support::*;

    /// A window so small that reading one large file fills it.
    fn tiny_window() -> glave_harness::Settings {
        let mut settings = settings();
        settings.compaction = Compaction {
            window_tokens: 1_000,
            at_percent: 80,
        };
        settings
    }

    fn text_of(message: &Message) -> String {
        let parts = match message {
            Message::User { parts }
            | Message::Tool { output: parts, .. }
            | Message::Assistant { parts, .. } => parts,
        };
        parts
            .iter()
            .filter_map(|part| match part {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn requests(seen: &Seen) -> Vec<ModelRequest> {
        seen.lock().unwrap().clone()
    }

    #[test]
    fn a_thread_that_outgrows_its_window_continues_from_a_summary() {
        let dir = folder("compact");
        std::fs::write(dir.join("big.txt"), "marigold\n".repeat(1_000)).unwrap();
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("read", json!({"path": "big.txt"}))],
                    StopReason::ToolUse,
                ),
                answer(
                    Some("big.txt repeats the word marigold."),
                    vec![],
                    StopReason::EndTurn,
                ),
                answer(Some("It repeats marigold."), vec![], StopReason::EndTurn),
            ];
            let log = InMemoryLog::new();
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, log, 1, tiny_window()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let mut shown = Vec::new();
            let summary = harness
                .run_turn(
                    &mut thread,
                    "what is in big.txt?",
                    &Cancel::new(),
                    &mut |live| {
                        shown.push(live);
                    },
                )
                .await
                .unwrap();

            let sent = requests(&seen);
            assert_eq!(sent.len(), 3, "the read, the summary, the answer");
            let instruction = text_of(sent[1].messages.last().unwrap());
            assert!(instruction.contains("handoff summary"), "{instruction}");
            assert!(
                !sent[1].tools.is_empty(),
                "tools stay listed for strict providers"
            );
            assert_eq!(
                sent[2].messages.len(),
                1,
                "the conversation restarts from one message"
            );
            let restart = text_of(&sent[2].messages[0]);
            assert!(restart.starts_with("[Earlier parts of this conversation were summarized"));
            assert!(restart.contains("big.txt repeats the word marigold."));
            assert!(
                restart.ends_with("word for word:\nwhat is in big.txt?"),
                "{restart}"
            );

            assert_eq!(summary.model_calls, 3, "the summary call counts");
            assert!(shown.iter().any(|live| matches!(
                live,
                Live::Compacted { before_tokens, after_tokens } if after_tokens < before_tokens
            )));
            let recorded = kinds(&events(harness.log(), thread.id).await);
            let compacted = recorded.iter().position(|kind| *kind == "thread.compacted");
            assert_eq!(
                recorded.get(compacted.unwrap() - 1),
                Some(&"model.responded"),
                "the summary call is recorded, content-free, before the compaction: {recorded:?}"
            );
            assert_eq!(recorded.last(), Some(&"turn.ended"));

            let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
            assert_eq!(
                resumed.messages(),
                thread.messages(),
                "the log alone rebuilds it"
            );
            assert_eq!(
                resumed.messages().len(),
                2,
                "the restart message and the answer"
            );
        });
    }

    #[test]
    fn a_thread_that_fits_is_left_alone() {
        let dir = folder("fits");
        std::fs::write(dir.join("small.txt"), "hello\n").unwrap();
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("read", json!({"path": "small.txt"}))],
                    StopReason::ToolUse,
                ),
                answer(Some("It says hello."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "read small.txt", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(requests(&seen).len(), 2);
            let recorded = kinds(&events(harness.log(), thread.id).await);
            assert!(!recorded.contains(&"thread.compacted"), "{recorded:?}");
        });
    }

    #[test]
    fn compact_on_request_restarts_from_the_summary() {
        let dir = folder("manual");
        block_on!(async {
            let answers = vec![
                answer(Some("Hi."), vec![], StopReason::EndTurn),
                answer(Some("We said hello."), vec![], StopReason::EndTurn),
            ];
            let (harness, _) =
                harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hello", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let compacted = harness
                .compact(&mut thread, &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert!(compacted.after_tokens > 0);
            assert_eq!(thread.messages().len(), 1);
            assert!(text_of(&thread.messages()[0]).contains("We said hello."));
            let recorded = kinds(&events(harness.log(), thread.id).await);
            assert_eq!(
                &recorded[recorded.len() - 2..],
                ["model.responded", "thread.compacted"]
            );
        });
    }

    #[test]
    fn an_empty_summary_leaves_the_thread_as_it_was() {
        let dir = folder("empty-summary");
        std::fs::write(dir.join("big.txt"), "marigold\n".repeat(1_000)).unwrap();
        block_on!(async {
            let answers = vec![
                answer(
                    None,
                    vec![("read", json!({"path": "big.txt"}))],
                    StopReason::ToolUse,
                ),
                answer(None, vec![], StopReason::EndTurn),
                answer(Some("It repeats marigold."), vec![], StopReason::EndTurn),
            ];
            let log = InMemoryLog::new();
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, log, 1, tiny_window()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let mut shown = Vec::new();
            harness
                .run_turn(
                    &mut thread,
                    "what is in big.txt?",
                    &Cancel::new(),
                    &mut |live| {
                        shown.push(live);
                    },
                )
                .await
                .unwrap();
            assert!(
                shown
                    .iter()
                    .any(|live| matches!(live, Live::CompactionFailed(_)))
            );
            let sent = requests(&seen);
            assert!(sent[2].messages.len() > 1, "the full conversation goes on");
            let recorded = kinds(&events(harness.log(), thread.id).await);
            assert!(!recorded.contains(&"thread.compacted"), "{recorded:?}");
        });
    }
}
