//! A model whose window is smaller than glave thought: a step refused as too long compacts the
//! thread and tries again; a summary refused as too long loses its oldest tool outputs first;
//! and a step still too long after that fails with the refusal.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod too_long {
    use std::collections::VecDeque;
    use std::future::{Future, ready};
    use std::sync::atomic::{AtomicI64, AtomicU64};
    use std::sync::{Arc, Mutex};

    use glave_event::{Outcome, Part, StopReason};
    use glave_harness::{AllowAll, Cancel, Harness, HarnessError, Parts, Workspace};
    use glave_ports::{
        Completion, Delta, Failure, MemoryBlobs, MemoryLog, Message, ModelProvider, ModelRequest,
        ProviderError,
    };
    use serde_json::json;

    use super::support::*;

    /// Answers with each scripted result in turn, keeping every request.
    struct Cramped {
        answers: Mutex<VecDeque<Result<Completion, ProviderError>>>,
        seen: Arc<Mutex<Vec<ModelRequest>>>,
    }

    impl ModelProvider for Cramped {
        fn complete(
            &self,
            request: &ModelRequest,
            _on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
            self.seen.lock().unwrap().push(request.clone());
            let next = self.answers.lock().unwrap().pop_front();
            ready(next.unwrap_or_else(|| Err(ProviderError::Protocol("script exhausted".into()))))
        }
    }

    fn too_long() -> Result<Completion, ProviderError> {
        Err(ProviderError::Http {
            status: 400,
            body: "This model's maximum context length is 4096 tokens. However, you requested \
                   9000 tokens."
                .into(),
        })
    }

    fn text(said: &str) -> Completion {
        answer(Some(said), vec![], StopReason::EndTurn)
    }

    /// A first turn that reads a big file.
    fn first_turn() -> Vec<Result<Completion, ProviderError>> {
        vec![
            Ok(answer(
                None,
                vec![("read", json!({"path": "big.txt"}))],
                StopReason::ToolUse,
            )),
            Ok(text("Read it.")),
        ]
    }

    /// A harness whose model gives `answers` in turn, in a folder holding a big file.
    async fn harness(
        name: &str,
        answers: Vec<Result<Completion, ProviderError>>,
    ) -> (
        Harness<Cramped, MemoryLog, MemoryBlobs, AllowAll>,
        Arc<Mutex<Vec<ModelRequest>>>,
    ) {
        let dir = folder(name);
        std::fs::write(dir.join("big.txt"), "word ".repeat(4_000)).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let parts = Parts {
            provider: Cramped {
                answers: Mutex::new(answers.into()),
                seen: Arc::clone(&seen),
            },
            log: MemoryLog::new(),
            blobs: MemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(Ticking(AtomicI64::new(1_791_000_000_000))),
            entropy: Box::new(Counting(AtomicU64::new(1))),
            workspace: Workspace::open(&dir, std::env::vars_os()).await.unwrap(),
        };
        (Harness::new(parts, settings()), seen)
    }

    fn removed(request: &ModelRequest) -> bool {
        request.messages.iter().any(|message| {
            matches!(message, Message::Tool { output, .. }
                if matches!(output.as_slice(), [Part::Text { text }] if text.starts_with("[This output was removed")))
        })
    }

    #[test]
    fn a_step_too_long_for_the_model_compacts_and_tries_again() {
        block_on(async {
            let mut answers = first_turn();
            answers.extend([
                too_long(),
                too_long(),
                Ok(text("Summary: read big.txt.")),
                Ok(text("Done.")),
            ]);
            let (harness, seen) = harness("too-long-retry", answers).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let cancel = Cancel::new();
            harness
                .run_turn(&mut thread, "read big.txt", &cancel, &mut |_| {})
                .await
                .unwrap();
            let turn = harness
                .run_turn(&mut thread, "and now?", &cancel, &mut |_| {})
                .await
                .unwrap();
            assert_eq!(turn.outcome, Outcome::Completed);
            let seen = seen.lock().unwrap();
            // Turn one (2), the refused step, two summaries, and the step again.
            assert_eq!(seen.len(), 6);
            assert!(!removed(&seen[3]), "the first summary asks with everything");
            assert!(removed(&seen[4]), "the second loses the oldest outputs");
            let restarted = format!("{:?}", seen[5].messages);
            assert_eq!(
                seen[5].messages.len(),
                1,
                "the step again, from the summary"
            );
            assert!(
                restarted.contains("Summary: read big.txt.") && restarted.contains("and now?"),
                "{restarted}"
            );
        });
    }

    #[test]
    fn a_step_still_too_long_after_compacting_fails_with_the_refusal() {
        block_on(async {
            let mut answers = first_turn();
            answers.extend([too_long(), Ok(text("Summary.")), too_long()]);
            let (harness, _) = harness("too-long-fails", answers).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let cancel = Cancel::new();
            harness
                .run_turn(&mut thread, "read big.txt", &cancel, &mut |_| {})
                .await
                .unwrap();
            let failed = harness
                .run_turn(&mut thread, "and now?", &cancel, &mut |_| {})
                .await;
            match failed {
                Err(HarnessError::Provider(error)) => assert_eq!(error.failure(), Failure::TooLong),
                other => panic!("expected the refusal, got {other:?}"),
            }
        });
    }

    #[test]
    fn a_first_message_too_long_is_not_tried_again() {
        block_on(async {
            let (harness, seen) = harness("too-long-first", vec![too_long()]).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let failed = harness
                .run_turn(
                    &mut thread,
                    "a very long paste",
                    &Cancel::new(),
                    &mut |_| {},
                )
                .await;
            assert!(matches!(failed, Err(HarnessError::Provider(_))));
            assert_eq!(seen.lock().unwrap().len(), 1, "asked once, not again");
        });
    }
}
