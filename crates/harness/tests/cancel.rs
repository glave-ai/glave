//! Cancelling a turn against a scripted model: before it starts, while the model answers, while
//! a command runs and while the person is asked. Each time the turn ends promptly, is recorded
//! as cancelled, and leaves a thread the next turn and the log can continue.

use std::future::{Future, pending};
use std::time::{Duration, Instant};

use glave_event::{Outcome, StopReason};
use glave_harness::{AllowAll, ApprovalRequest, Approver, Cancel, Decision, Harness, Parts};
use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{
    Completion, Delta, InMemoryBlobs, InMemoryLog, Message, ModelProvider, ModelRequest,
    ProviderError,
};
use serde_json::json;

use crate::support::*;

/// The model told to the next turn after a cancel.
const NOTE: &str = "[The person cancelled this turn on purpose.";

/// A model that cancels the turn as soon as it is asked, then never answers.
struct Stalls(Cancel);

impl ModelProvider for Stalls {
    fn complete(
        &self,
        _request: &ModelRequest,
        _on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        self.0.cancel();
        pending()
    }
}

/// A person who cancels the turn when asked, then never answers.
struct WalksAway(Cancel);

impl Approver for WalksAway {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        self.0.cancel();
        pending()
    }
}

#[test]
fn a_turn_cancelled_before_it_starts_asks_nothing_and_the_next_turn_carries_on() {
    let dir = folder("cancel-early");
    block_on!(async {
        let answers = vec![answer(Some("Here now."), vec![], StopReason::EndTurn)];
        let (harness, seen) =
            harness_seeing(&dir, answers, AllowAll, InMemoryLog::new(), 1, settings()).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        cancel.cancel();
        let summary = harness
            .run_turn(&mut thread, "first", &cancel, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Cancelled);
        assert_eq!(summary.model_calls, 0);
        assert!(seen.lock().unwrap().is_empty(), "the model was never asked");
        let recorded = kinds(&events(harness.log(), thread.id).await);
        assert_eq!(
            recorded,
            [
                "thread.started",
                "turn.started",
                "message.added",
                "message.added",
                "turn.ended"
            ]
        );

        let summary = turn(&harness, &mut thread, "second").await;
        assert_eq!(summary.outcome, Outcome::Completed);
        let sent = seen.lock().unwrap().clone();
        let said: Vec<String> = sent[0].messages.iter().map(text_of).collect();
        assert_eq!(said.len(), 3, "{said:?}");
        assert_eq!(said[0], "first");
        assert!(said[1].starts_with(NOTE), "{said:?}");
        assert_eq!(said[2], "second");
        let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert_eq!(resumed.messages(), thread.messages());
    });
}

#[test]
fn a_model_that_never_answers_is_abandoned() {
    let dir = folder("cancel-model");
    block_on!(async {
        let cancel = Cancel::new();
        let parts = Parts {
            provider: Stalls(cancel.clone()),
            log: InMemoryLog::new(),
            blobs: InMemoryBlobs::new(),
            approver: AllowAll,
            clock: Box::new(ManualClock::ticking(1_791_000_000_000)),
            entropy: Box::new(CountingEntropy::starting_at(1)),
            workspace: workspace(&dir).await,
        };
        let harness = Harness::new(parts, settings());
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let summary = harness
            .run_turn(&mut thread, "hello", &cancel, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Cancelled);
        let last = thread.messages().last().unwrap();
        assert!(text_of(last).starts_with(NOTE));
        let recorded = kinds(&events(harness.log(), thread.id).await);
        assert_eq!(recorded.last(), Some(&"turn.ended"));
        assert!(
            !recorded.contains(&"model.responded"),
            "nothing half-answered is recorded: {recorded:?}"
        );
    });
}

#[cfg(unix)]
#[test]
fn a_running_command_stops_with_what_it_started_and_later_calls_do_not_run() {
    let dir = folder("cancel-command");
    // The command says through a pipe when its child is running; only then is the turn
    // cancelled, so the test never races the shell.
    let ready = dir.join("ready");
    let made = std::process::Command::new("mkfifo")
        .arg(&ready)
        .status()
        .unwrap();
    assert!(made.success());
    block_on!(async {
        let calls = vec![
            (
                "bash",
                json!({"command": "sleep 30 & echo $! > child.pid; echo up > ready; wait"}),
            ),
            ("write", json!({"path": "after.txt", "content": "no"})),
        ];
        let answers = vec![answer(None, calls, StopReason::ToolUse)];
        let harness = harness(&dir, answers, AllowAll).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let cancel = Cancel::new();
        let canceller = cancel.clone();
        let waiter = tokio::task::spawn_blocking(move || {
            let _ = std::fs::read_to_string(ready);
            canceller.cancel();
        });
        let started = Instant::now();
        let summary = harness
            .run_turn(&mut thread, "go", &cancel, &mut |_| {})
            .await
            .unwrap();
        waiter.await.unwrap();
        assert_eq!(summary.outcome, Outcome::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "stopped promptly, not after the 30 s sleep"
        );
        let results: Vec<String> = thread
            .messages()
            .iter()
            .filter(|message| matches!(message, Message::Tool { .. }))
            .map(text_of)
            .collect();
        assert_eq!(results.len(), 2, "every call has a result: {results:?}");
        assert!(
            results[0].contains("the person cancelled the turn"),
            "{results:?}"
        );
        assert_eq!(results[1], "Not run: the person cancelled the turn.");
        assert!(!dir.join("after.txt").exists());
        let child = std::fs::read_to_string(dir.join("child.pid")).unwrap();
        let alive = std::process::Command::new("kill")
            .args(["-0", child.trim()])
            .status()
            .unwrap();
        assert!(!alive.success(), "the command's own child was stopped too");
    });
}

#[test]
fn an_unanswered_question_is_abandoned_and_the_action_never_runs() {
    let dir = folder("cancel-ask");
    block_on!(async {
        let cancel = Cancel::new();
        let calls = vec![("bash", json!({"command": "touch ran.txt"}))];
        let answers = vec![answer(None, calls, StopReason::ToolUse)];
        let harness = asking_harness(&dir, answers, WalksAway(cancel.clone())).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let summary = harness
            .run_turn(&mut thread, "go", &cancel, &mut |_| {})
            .await
            .unwrap();
        assert_eq!(summary.outcome, Outcome::Cancelled);
        assert!(!dir.join("ran.txt").exists());
        let resumed = harness.resume_thread(scope(), thread.id).await.unwrap();
        assert_eq!(
            resumed.messages(),
            thread.messages(),
            "the log alone rebuilds it"
        );
    });
}
