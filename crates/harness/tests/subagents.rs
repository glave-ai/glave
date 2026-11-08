//! Subagents: a call that hands work to a child thread, which starts fresh with only the task,
//! works with the parent's tools or fewer, and reports back; several run side by side.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use glave_event::{Body, Outcome, StopReason};
use glave_harness::{AllowAll, Cancel, Decision, Harness, HarnessError, Live, Parts};
use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{
    Caller, Completion, Delta, InMemoryBlobs, InMemoryLog, ModelProvider, ModelRequest,
    ProviderError,
};
use serde_json::json;
use tokio::sync::Barrier;

use crate::network::InTheLog;
use crate::support::*;

fn with_subagents() -> glave_harness::Settings {
    let mut settings = settings();
    settings.subagents = true;
    settings
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
        turn(&harness, &mut thread, "do x").await;
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
        turn(&harness, &mut thread, "hi").await;
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
        let summary = turn(&harness, &mut thread, "do both").await;
        assert_eq!(summary.outcome, Outcome::Completed);
        assert_eq!(
            results(thread.messages()),
            ["Done side by side.", "Done side by side."]
        );
    });
}

/// What a parent that has a child write x.txt is answered, and the child: the parent's call,
/// the child's write, which asks in Ask first, and its last word, then the parent's.
pub(crate) fn a_child_writes() -> Vec<Completion> {
    let brief = json!({"description": "write it", "prompt": "Write x.txt."});
    let write = json!({"path": "x.txt", "content": "x"});
    vec![
        answer(None, vec![("agent", brief)], StopReason::ToolUse),
        answer(None, vec![("write", write)], StopReason::ToolUse),
        answer(Some("Wrote it."), vec![], StopReason::EndTurn),
        answer(Some("The child wrote it."), vec![], StopReason::EndTurn),
    ]
}

/// Ask first, with subagents.
pub(crate) fn asking_with_subagents() -> glave_harness::Settings {
    let mut settings = asking();
    settings.subagents = true;
    settings
}

/// Runs a turn of `thread` that has a child write x, until the child waits for its approval
/// in the log, then allows it there, as a server does: the turn goes on from the log.
pub(crate) async fn until_the_child_is_allowed<P, L, B, A>(
    harness: &Harness<P, L, B, A>,
    thread: &mut glave_harness::Thread,
) where
    P: ModelProvider,
    L: glave_ports::EventLog,
    B: glave_ports::BlobStore,
    A: glave_harness::Approver,
{
    let started = Mutex::new(None);
    let mut live = |update: Live| {
        if let Live::SubagentStarted { thread, .. } = update {
            *started.lock().unwrap() = Some(thread);
        }
    };
    let stopped = harness
        .run_turn(thread, "have a child write x", &Cancel::new(), &mut live)
        .await;
    assert!(
        matches!(stopped, Err(HarnessError::Waits(_))),
        "{stopped:?}"
    );
    let child = started.lock().unwrap().unwrap();
    let call = events(harness.log(), child)
        .await
        .iter()
        .find_map(|event| match &event.body {
            Body::ApprovalRequested(asked) => Some(asked.call_id.clone()),
            _ => None,
        });
    harness
        .decide(scope(), child, &call.unwrap(), Decision::AllowOnce, None)
        .await
        .unwrap();
}

#[test]
fn a_child_that_waited_counts_once_in_its_parent_s_turn() {
    let dir = folder("subagents-waited");
    block_on!(async {
        let log = InMemoryLog::new();
        let settings = asking_with_subagents();
        let answers = a_child_writes();
        let (harness, _) = harness_seeing(&dir, answers, InTheLog, log, 1, settings).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        until_the_child_is_allowed(&harness, &mut thread).await;
        let turn = harness
            .go_on(&mut thread, &Caller::default(), &Cancel::new(), &mut |_| {})
            .await
            .unwrap();
        // Two model calls each, and one call each: the parent's agent and the child's write.
        let counted = (turn.model_calls, turn.tool_calls, turn.tokens.input);
        assert_eq!(counted, (4, 2, 400), "{turn:?}");
        let ended = events(harness.log(), thread.id).await;
        let ended = ended.iter().find_map(|event| match &event.body {
            Body::TurnEnded(ended) => Some(ended.clone()),
            _ => None,
        });
        assert_eq!(ended, Some(turn), "the log says the same");
    });
}
