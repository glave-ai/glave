//! Subagents: a call that hands work to a child thread, which starts fresh with only the task,
//! works with the parent's tools or fewer, and reports back; several run side by side.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_event::{Body, Outcome, Part, StopReason};
use glave_harness::{AllowAll, Cancel, Harness, Live, Parts};
use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{
    Completion, Delta, InMemoryBlobs, InMemoryLog, Message, ModelProvider, ModelRequest,
    ProviderError,
};
use serde_json::json;
use tokio::sync::Barrier;

use crate::support::*;

fn with_subagents() -> glave_harness::Settings {
    let mut settings = settings();
    settings.subagents = true;
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

/// The child's first event names its parent's call, says a subagent started it, and works
/// for the same person as the parent.
async fn started_by_its_parent<L: glave_ports::EventLog>(
    log: &L,
    child: glave_event::ThreadId,
    parent: glave_event::ThreadId,
) {
    let (Body::ThreadStarted(started), Body::ThreadStarted(parents)) = (
        &events(log, child).await[0].body,
        &events(log, parent).await[0].body,
    ) else {
        panic!("a thread did not start");
    };
    assert_eq!(started.parent.as_ref().unwrap().thread, parent);
    assert_eq!(started.origin.kind, glave_event::OriginKind::Subagent);
    assert_eq!(
        started.for_, parents.for_,
        "a subagent works for the same person"
    );
}

#[test]
fn a_child_does_the_task_in_its_own_thread_and_reports_back() {
    let dir = folder("subagents-report");
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![(
                    "agent",
                    json!({"description": "find the tests", "prompt": "List the test files.", "subagent_type": "explore"}),
                )],
                StopReason::ToolUse,
            ),
            answer(Some("Found tests/a.rs."), vec![], StopReason::EndTurn),
            answer(Some("There is one test file."), vec![], StopReason::EndTurn),
        ];
        let log = InMemoryLog::new();
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, log, 1, with_subagents()).await;
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
        assert!(names(&seen[0]).contains(&"agent".to_owned()));
        assert_eq!(
            names(&seen[1]),
            ["search", "glob", "read"],
            "an explorer only reads, and starts no subagents"
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
                Live::SubagentStarted {
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
            |live| matches!(live, Live::SubagentEnded { thread, ok: true } if *thread == child)
        ));
        started_by_its_parent(harness.log(), child, thread.id).await;
    });
}

#[test]
fn a_general_child_has_the_parents_tools_but_no_subagents() {
    let dir = folder("subagents-general");
    block_on!(async {
        let answers = vec![
            answer(
                None,
                vec![("agent", json!({"description": "x", "prompt": "Do x."}))],
                StopReason::ToolUse,
            ),
            answer(Some("Did x."), vec![], StopReason::EndTurn),
            answer(Some("Done."), vec![], StopReason::EndTurn),
        ];
        let (harness, seen) = harness_seeing(
            &dir,
            answers,
            AllowAll,
            InMemoryLog::new(),
            1,
            with_subagents(),
        )
        .await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "do x", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(
            names(&seen[1]),
            [
                "search",
                "glob",
                "read",
                "write",
                "edit",
                "bash",
                "command_output"
            ]
        );
    });
}

#[test]
fn without_the_setting_there_is_no_agent_tool() {
    let dir = folder("subagents-off");
    block_on!(async {
        let answers = vec![answer(Some("Hi."), vec![], StopReason::EndTurn)];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        harness
            .run_turn(&mut thread, "hi", &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        assert!(!names(&seen.lock().unwrap()[0]).contains(&"agent".to_owned()));
    });
}

/// A model whose children each wait until both have asked: only subagents running side by side
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
                Err(_) => Err(ProviderError::Protocol(
                    "the subagents ran one by one".into(),
                )),
            }
        }
    }
}

#[test]
fn subagents_in_one_response_run_side_by_side() {
    let dir = folder("subagents-together");
    block_on!(async {
        let task = |name: &str| {
            (
                "agent",
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
            log: InMemoryLog::new(),
            blobs: InMemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(ManualClock::ticking(1_791_000_000_000)),
            entropy: Box::new(CountingEntropy::starting_at(1)),
            workspace: workspace(&dir).await,
        };
        let harness = Harness::new(parts, with_subagents());
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
