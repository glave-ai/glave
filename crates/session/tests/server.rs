//! The session server in-process, beyond what the conformance suite checks on every transport:
//! one turn at a time (a message sent meanwhile is queued to it, `steer.rs`), cancelling a
//! running turn, a retried turn following the first, an approval with no one to answer it, a
//! fresh server picking a thread up from the log alone, stopping with a grace, and the shape
//! of the messages.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{Body, Outcome, Verdict, decode};
use glave_session::Answering;
use glave_session::protocol::{
    Answer, IdempotencyKey, Meta, Notification, Reply, Request, Status, Update, code,
};
use jiff::Timestamp;
use serde_json::json;
use tokio::sync::Notify;

use crate::support::*;

/// A server whose model waits at `gate` before its one answer.
async fn gated(dir: &std::path::Path, gate: &Arc<Notify>) -> Tested {
    let mut setup = Setup::asking(vec![answer(Some("Slow."), None)], Answering::NoOne);
    setup.gate = Some(Arc::clone(gate));
    setup.build(dir).await.0
}

#[test]
fn one_turn_runs_at_a_time_and_cancel_ends_it() {
    let dir = folder("one-at-a-time");
    block_on!(async {
        let server = gated(&dir, &Arc::new(Notify::new())).await;
        let thread = start(&server, false).await;
        let mut first = stream_turn(&server, thread, "slow", None).await.unwrap();
        // A second message is queued to the running turn, not a turn of its own.
        let second = server.request(turn_start(thread, "again", None)).await;
        assert!(matches!(second, Ok(Reply::Queued { .. })), "{second:?}");
        // A client that follows only turns it starts is told so.
        let followed = stream_turn(&server, thread, "and again", None).await;
        assert_eq!(followed.err().map(|e| e.code), Some(code::TURN_RUNNING));
        match server.request(Request::ThreadGet { thread }).await.unwrap() {
            Reply::Thread { thread } => assert_eq!(thread.status, Status::Running),
            other => panic!("{other:?}"),
        }
        let cancelled = server
            .request(Request::TurnCancel { thread })
            .await
            .unwrap();
        assert!(matches!(cancelled, Reply::Done {}));
        let ended = first.finish().await.unwrap();
        assert_eq!(ended.outcome, Outcome::Cancelled);
        let again = server.request(Request::TurnCancel { thread }).await;
        assert_eq!(again.err().map(|e| e.code), Some(code::NO_TURN));
    });
}

#[test]
fn a_stop_interrupts_a_turn_still_running_after_its_grace_and_starts_nothing_new() {
    let dir = folder("stop-interrupts");
    block_on!(async {
        let server = gated(&dir, &Arc::new(Notify::new())).await;
        let thread = start(&server, false).await;
        let mut running = stream_turn(&server, thread, "slow", None).await.unwrap();
        server.stop(Duration::ZERO).await;
        let ended = running.finish().await.unwrap();
        assert_eq!(
            ended.outcome,
            Outcome::Interrupted,
            "the person did not stop it"
        );
        let refused = server.request(turn_start(thread, "again", None)).await;
        assert_eq!(refused.err().map(|e| e.code), Some(code::NOT_AVAILABLE));
    });
}

#[test]
fn a_turn_that_ends_within_the_grace_ends_as_it_would_have() {
    let dir = folder("stop-grace");
    block_on!(async {
        let gate = Arc::new(Notify::new());
        let server = gated(&dir, &gate).await;
        let thread = start(&server, false).await;
        let mut running = stream_turn(&server, thread, "slow", None).await.unwrap();
        // The stop starts first, then the model answers: well within the grace.
        tokio::join!(server.stop(Duration::from_secs(3_600)), async {
            gate.notify_one();
        });
        assert_eq!(running.finish().await.unwrap().outcome, Outcome::Completed);
    });
}

#[test]
fn a_retry_with_the_same_key_follows_the_running_turn_instead_of_starting_one() {
    let dir = folder("retry-running");
    block_on!(async {
        let gate = Arc::new(Notify::new());
        let server = gated(&dir, &gate).await;
        let thread = start(&server, false).await;
        let key = IdempotencyKey::new("attempt-1").unwrap();
        let attempt =
            |key: &IdempotencyKey| stream_turn(&server, thread, "slow", Some(key.clone()));
        let mut first = attempt(&key).await.unwrap();
        let mut retried = attempt(&key).await.unwrap();
        // Another key's message is queued to the running turn instead.
        let other = IdempotencyKey::new("attempt-2").unwrap();
        let queued = server
            .request(turn_start(thread, "slow", Some(other)))
            .await;
        assert!(matches!(queued, Ok(Reply::Queued { .. })), "{queued:?}");
        gate.notify_one();
        let (first, retried) = (first.finish().await, retried.finish().await);
        assert_eq!(first.unwrap(), retried.unwrap());
        // Once ended, the key still names that turn.
        let mut late = attempt(&key).await.unwrap();
        assert_eq!(late.finish().await.unwrap().outcome, Outcome::Completed);
        assert!(IdempotencyKey::new("").is_err());
        assert!(IdempotencyKey::new("k".repeat(256)).is_err());
    });
}

#[test]
fn with_no_one_to_answer_the_call_is_not_run() {
    let dir = folder("nobody");
    block_on!(async {
        let answers = vec![
            answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
            answer(Some("I could not."), None),
        ];
        let server = server(&dir, answers, Answering::NoOne).await;
        let thread = start(&server, false).await;
        let reply = turn(&server, thread, "touch it").await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
        assert!(!dir.join("ran.txt").exists());
        let decided: Vec<Verdict> = events(&server, thread)
            .await
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::ApprovalDecided(decided) => Some(decided.decision),
                _ => None,
            })
            .collect();
        assert_eq!(decided, [Verdict::NoOneThere]);
    });
}

#[test]
fn a_turn_answered_by_no_one_declines_what_asks_where_clients_answer_and_the_next_asks_again() {
    let dir = folder("unattended");
    block_on!(async {
        let answers = vec![
            answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
            answer(Some("I could not."), None),
            answer(Some("Hi."), None),
        ];
        let (server, seen) = Setup::asking(answers, Answering::Clients).build(&dir).await;
        let thread = start(&server, false).await;
        let unattended = Request::TurnStart {
            thread,
            prompt: "touch it".into(),
            background: false,
            budget: None,
            model: None,
            approval: None,
            effort: None,
            unattended: true,
            meta: Meta::default(),
            files: Vec::new(),
        };
        let reply = server.request(unattended).await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
        assert!(!dir.join("ran.txt").exists());
        let decided: Vec<Verdict> = events(&server, thread)
            .await
            .into_iter()
            .filter_map(|raw| match decode(raw).unwrap().body {
                Body::ApprovalDecided(decided) => Some(decided.decision),
                _ => None,
            })
            .collect();
        assert_eq!(decided, [Verdict::NoOneThere]);
        turn(&server, thread, "hello").await.unwrap();
        let offered: Vec<bool> = json(&seen)
            .iter()
            .map(|request| request.contains("\"name\":\"question\""))
            .collect();
        assert_eq!(
            offered,
            [false, false, true],
            "no question while no one answers, then offered again"
        );
    });
}

#[test]
fn a_fresh_server_continues_a_thread_from_the_log_alone() {
    let dir = folder("stateless");
    block_on!(async {
        let database = dir.join("glave.db");
        let (first, _) = Setup::asking(vec![answer(Some("One."), None)], Answering::NoOne)
            .build_on(&dir, sqlite(&database).await.log())
            .await;
        let thread = start(&first, false).await;
        turn(&first, thread, "one").await.unwrap();
        first.shutdown().await;

        let mut again = Setup::asking(vec![answer(Some("Two."), None)], Answering::NoOne);
        again.seed = 2;
        let (second, _) = again.build_on(&dir, sqlite(&database).await.log()).await;
        let reply = turn(&second, thread, "two").await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
        let Reply::Events { events, next } = second
            .request(Request::ThreadEvents {
                thread,
                after: None,
                limit: Some(3),
            })
            .await
            .unwrap()
        else {
            panic!("expected events");
        };
        assert_eq!(events.len(), 3);
        assert_eq!(
            next,
            events.last().map(|e| e.seq),
            "a full page says where to go on"
        );
    });
}

#[test]
fn messages_have_the_shape_the_spec_gives() {
    let thread = glave_event::ThreadId::new(Timestamp::UNIX_EPOCH, [1; 10]).unwrap();
    let request = Request::TurnStart {
        background: false,
        budget: None,
        thread,
        prompt: "hi".into(),
        meta: Meta::default(),
        model: None,
        approval: None,
        effort: None,
        unattended: false,
        files: Vec::new(),
    };
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(wire["method"], "turn/start");
    assert_eq!(wire["params"]["prompt"], "hi");
    let parsed: Request = serde_json::from_value(json!({
        "method": "approval/decide",
        "params": {"thread": thread, "approval": "call_7", "decision": "allow_for_thread",
                   "_meta": {"protocol": "2026-10"}}
    }))
    .unwrap();
    assert!(matches!(
        parsed,
        Request::ApprovalDecide {
            decision: Answer::AllowForThread,
            ..
        }
    ));
    let live = Notification::Live {
        thread,
        update: Update::Retrying {
            retry: 1,
            of: 4,
            wait_ms: 1000,
            error: "503".into(),
        },
    };
    let wire = serde_json::to_value(&live).unwrap();
    assert_eq!(wire["method"], "turn/live");
    assert_eq!(wire["params"]["update"]["kind"], "retrying");
}
