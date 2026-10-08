//! Tasks: a call that hands work to a child thread, which starts fresh with only the task,
//! works with the parent's tools or fewer, and reports back; several run side by side.

#[cfg(test)]
#[path = "support/script.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod tasks {
    use std::future::Future;
    use std::sync::atomic::{AtomicI64, AtomicU64};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use glave_event::{Body, Outcome, Part, StopReason};
    use glave_harness::{AllowAll, Cancel, Harness, Live, Parts, Workspace};
    use glave_ports::{
        Completion, Delta, MemoryBlobs, MemoryLog, Message, ModelProvider, ModelRequest,
        ProviderError,
    };
    use serde_json::json;
    use tokio::sync::Barrier;

    use super::support::*;

    fn with_tasks() -> glave_harness::Settings {
        let mut settings = settings();
        settings.tasks = true;
        settings
    }

    fn results(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::Tool { output, .. } => output.iter().find_map(|part| match part {
                    Part::Text { text } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect()
    }

    fn names(request: &ModelRequest) -> Vec<String> {
        request.tools.iter().map(|spec| spec.name.clone()).collect()
    }

    #[test]
    fn a_child_does_the_task_in_its_own_thread_and_reports_back() {
        let dir = folder("tasks-report");
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![(
                        "task",
                        json!({"description": "find the tests", "prompt": "List the test files.", "agent": "explore"}),
                    )],
                    StopReason::ToolUse,
                ),
                answer(Some("Found tests/a.rs."), vec![], StopReason::EndTurn),
                answer(Some("There is one test file."), vec![], StopReason::EndTurn),
            ];
            let log = MemoryLog::new();
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, log, 1, with_tasks()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let mut shown = Vec::new();
            let summary = harness
                .run_turn(
                    &mut thread,
                    "how many test files?",
                    &Cancel::new(),
                    &mut |live| {
                        shown.push(live);
                    },
                )
                .await
                .unwrap();
            assert_eq!(summary.outcome, Outcome::Completed);
            assert_eq!(
                summary.model_calls, 3,
                "the child's call counts in the turn"
            );
            assert_eq!(results(thread.messages()), ["Found tests/a.rs."]);
            let seen = seen.lock().unwrap().clone();
            assert!(names(&seen[0]).contains(&"task".to_owned()));
            assert_eq!(
                names(&seen[1]),
                ["search", "glob", "read"],
                "an explorer only reads, and starts no tasks"
            );
            let brief = format!("{:?}", seen[1].messages);
            assert!(brief.contains("List the test files."), "{brief}");
            assert!(
                !brief.contains("how many test files?"),
                "the child starts fresh"
            );
            let child = shown
                .iter()
                .find_map(|live| match live {
                    Live::TaskStarted {
                        thread,
                        description,
                    } => {
                        assert_eq!(description, "find the tests");
                        Some(*thread)
                    }
                    _ => None,
                })
                .unwrap();
            assert!(
                shown
                    .iter()
                    .any(|live| matches!(live, Live::Child { thread, .. } if *thread == child))
            );
            assert!(shown.iter().any(
                |live| matches!(live, Live::TaskEnded { thread, ok: true } if *thread == child)
            ));
            let started = events(harness.log(), child).await;
            match &started[0].body {
                Body::ThreadStarted(started) => {
                    let parent = started.parent.as_ref().unwrap();
                    assert_eq!(parent.thread, thread.id);
                }
                other => panic!("{other:?}"),
            }
        });
    }

    #[test]
    fn a_general_child_has_the_parents_tools_but_no_tasks() {
        let dir = folder("tasks-general");
        block_on(async {
            let answers = vec![
                answer(
                    None,
                    vec![("task", json!({"description": "x", "prompt": "Do x."}))],
                    StopReason::ToolUse,
                ),
                answer(Some("Did x."), vec![], StopReason::EndTurn),
                answer(Some("Done."), vec![], StopReason::EndTurn),
            ];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, with_tasks()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "do x", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            let seen = seen.lock().unwrap();
            assert_eq!(
                names(&seen[1]),
                ["search", "glob", "read", "write", "edit", "bash"]
            );
        });
    }

    #[test]
    fn without_the_setting_there_is_no_task_tool() {
        let dir = folder("tasks-off");
        block_on(async {
            let answers = vec![answer(Some("Hi."), vec![], StopReason::EndTurn)];
            let (harness, seen) =
                harness_seeing(&dir, answers, AllowAll, MemoryLog::new(), 1, settings()).await;
            let mut thread = harness.start_thread(scope()).await.unwrap();
            harness
                .run_turn(&mut thread, "hi", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert!(!names(&seen.lock().unwrap()[0]).contains(&"task".to_owned()));
        });
    }

    /// A model whose children each wait until both have asked: only tasks running side by side
    /// get past it.
    struct Together {
        barrier: Arc<Barrier>,
        parent: Mutex<Vec<Completion>>,
    }

    impl ModelProvider for Together {
        fn complete(
            &self,
            request: &ModelRequest,
            _on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
            let child = format!("{:?}", request.messages).contains("You are a helper");
            let barrier = Arc::clone(&self.barrier);
            let next = (!child).then(|| self.parent.lock().unwrap().remove(0));
            async move {
                if let Some(answer) = next {
                    return Ok(answer);
                }
                let met = tokio::time::timeout(Duration::from_secs(10), barrier.wait()).await;
                match met {
                    Ok(_) => Ok(answer(
                        Some("Done side by side."),
                        vec![],
                        StopReason::EndTurn,
                    )),
                    Err(_) => Err(ProviderError::Protocol("the tasks ran one by one".into())),
                }
            }
        }
    }

    #[test]
    fn tasks_in_one_response_run_side_by_side() {
        let dir = folder("tasks-together");
        block_on(async {
            let task = |name: &str| {
                (
                    "task",
                    json!({"description": name, "prompt": format!("Do {name}.")}),
                )
            };
            let parent = vec![
                answer(None, vec![task("one"), task("two")], StopReason::ToolUse),
                answer(Some("Both done."), vec![], StopReason::EndTurn),
            ];
            let parts = Parts {
                provider: Together {
                    barrier: Arc::new(Barrier::new(2)),
                    parent: Mutex::new(parent),
                },
                log: MemoryLog::new(),
                blobs: MemoryBlobs::new(),
                approver: AllowAll,
                clock: Box::new(Ticking(AtomicI64::new(1_791_000_000_000))),
                entropy: Box::new(Counting(AtomicU64::new(1))),
                workspace: Workspace::open(&dir, std::env::vars_os()).await.unwrap(),
            };
            let harness = Harness::new(parts, with_tasks());
            let mut thread = harness.start_thread(scope()).await.unwrap();
            let summary = harness
                .run_turn(&mut thread, "do both", &Cancel::new(), &mut |_| {})
                .await
                .unwrap();
            assert_eq!(summary.outcome, Outcome::Completed);
            assert_eq!(
                results(thread.messages()),
                ["Done side by side.", "Done side by side."]
            );
        });
    }
}
