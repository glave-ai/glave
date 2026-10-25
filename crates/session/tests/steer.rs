//! Steering a running turn over the session API: `turn/start` while a turn runs is answered at
//! once as queued, and the model reads the message at its next call, recorded under the id the
//! reply gave; a retry with the same idempotency key is answered with that id and queues
//! nothing more; a message sent during the model's last call starts the next turn in the same
//! stream; the queue is bounded; a message chooses no model; Stop gives back what waited, and a
//! message sent once it is stopped starts the next turn.

use std::sync::Arc;

use glave_event::{Body, EventId, Outcome, Role, ThreadId, decode};
use glave_ports::Caller;
use glave_session::protocol::{IdempotencyKey, Notification, Reply, Request, Update, code};
use glave_session::{Answered, Answering, TurnItem, TurnStream};
use serde_json::json;
use tokio::sync::Notify;

use crate::support::*;

/// A server whose model waits at `gate` before each of `answers`, asking no one.
async fn gated(
    dir: &std::path::Path,
    answers: Vec<glave_ports::Completion>,
) -> (Tested, Seen, Arc<Notify>) {
    let gate = Arc::new(Notify::new());
    let mut setup = Setup::asking(answers, Answering::NoOne);
    setup.gate = Some(Arc::clone(&gate));
    let (server, seen) = setup.build(dir).await;
    (server, seen, gate)
}

fn send(thread: ThreadId, prompt: &str) -> Request {
    turn_start(thread, prompt, None)
}

/// Sends `prompt` and expects it queued: the id its message will be recorded under.
async fn queue(server: &Tested, thread: ThreadId, prompt: &str) -> EventId {
    match server.call(&Caller::default(), send(thread, prompt)).await {
        Answered::Reply(Ok(Reply::Queued { queued })) => queued.id,
        Answered::Reply(other) => panic!("queued, not {other:?}"),
        Answered::Stream(_) => panic!("queued, not a turn of its own"),
        Answered::Watch(_) | Answered::List(_) => panic!("queued, not a watch"),
    }
}

/// Follows `stream` to its end: the ids of the person's messages it carried, those given back
/// unread, and how it ended.
async fn follow(stream: &mut TurnStream) -> (Vec<EventId>, Vec<EventId>, Result<Outcome, i64>) {
    let (mut read, mut unread) = (Vec::new(), Vec::new());
    loop {
        match stream.next().await {
            Some(TurnItem::Note(note)) => match note.as_ref() {
                Notification::Event { event, .. } => {
                    if let Body::MessageAdded(added) = decode(event.clone()).unwrap().body
                        && added.role == Role::User
                    {
                        read.push(event.id);
                    }
                }
                Notification::Live {
                    update: Update::Unread { id, .. },
                    ..
                } => unread.push(*id),
                _ => {}
            },
            Some(TurnItem::Ended(ended)) => {
                return (read, unread, ended.map(|t| t.outcome).map_err(|e| e.code));
            }
            Some(TurnItem::Missed(_)) | None => panic!("the stream ended early"),
        }
    }
}

/// The text of the person's messages the model was sent in its `call`th request.
fn person_said(seen: &Seen, call: usize) -> Vec<String> {
    let request: serde_json::Value = serde_json::from_str(&json(seen)[call]).unwrap();
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .map(|message| {
            let parts = message["parts"].as_array().unwrap();
            parts.last().unwrap()["text"].as_str().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn a_message_sent_while_a_turn_runs_is_read_at_the_models_next_call() {
    let dir = folder("steer-queued");
    block_on!(async {
        let answers = vec![
            answer(None, Some(("bash", json!({"command": "echo one"})))),
            answer(Some("Done, with 4000."), None),
        ];
        let (server, seen, gate) = gated(&dir, answers).await;
        let thread = start(&server, false).await;
        let mut first = stream_turn(&server, thread, "set the port to 3000", None)
            .await
            .unwrap();
        asked(&seen, 1).await;
        let id = queue(&server, thread, "actually, 4000").await;
        gate.notify_one();
        asked(&seen, 2).await;
        gate.notify_one();
        let (read, unread, ended) = follow(&mut first).await;
        assert_eq!(ended, Ok(Outcome::Completed));
        assert_eq!(read.last(), Some(&id), "its thread/event says it was read");
        assert_eq!(unread, []);
        assert_eq!(
            person_said(&seen, 1).last().map(String::as_str),
            Some("actually, 4000"),
            "after the command's result, in the same turn"
        );
        let turns = kinds(&server, thread).await;
        assert_eq!(turns.iter().filter(|k| *k == "turn.started").count(), 1);
    });
}

#[test]
fn a_retried_message_is_answered_with_its_first_answer_and_read_once() {
    let dir = folder("steer-retried");
    block_on!(async {
        let answers = vec![
            answer(None, Some(("bash", json!({"command": "echo one"})))),
            answer(Some("Done."), None),
        ];
        let (server, seen, gate) = gated(&dir, answers).await;
        let thread = start(&server, false).await;
        let mut first = stream_turn(&server, thread, "go", None).await.unwrap();
        asked(&seen, 1).await;
        let key = IdempotencyKey::new("message-1").unwrap();
        let mut ids = Vec::new();
        // Sent, then sent again as a client retries when it got no answer.
        for _ in 0..2 {
            let sent = server
                .request(turn_start(thread, "and this", Some(key.clone())))
                .await;
            let Ok(Reply::Queued { queued }) = sent else {
                panic!("queued, not {sent:?}")
            };
            ids.push(queued.id);
        }
        assert_eq!(
            ids[0], ids[1],
            "the retry is answered as the first send was"
        );
        gate.notify_one();
        asked(&seen, 2).await;
        gate.notify_one();
        let (read, unread, ended) = follow(&mut first).await;
        assert_eq!(ended, Ok(Outcome::Completed));
        assert_eq!(unread, []);
        assert_eq!(read.iter().filter(|id| **id == ids[0]).count(), 1);
        assert_eq!(person_said(&seen, 1), ["go", "and this"], "read once");
    });
}

#[test]
fn a_message_sent_during_the_last_call_starts_the_next_turn_in_the_same_stream() {
    let dir = folder("steer-late");
    block_on!(async {
        let answers = vec![answer(Some("First."), None), answer(Some("Second."), None)];
        let (server, seen, gate) = gated(&dir, answers).await;
        let thread = start(&server, false).await;
        let mut first = stream_turn(&server, thread, "one", None).await.unwrap();
        asked(&seen, 1).await;
        let id = queue(&server, thread, "two").await;
        gate.notify_one();
        asked(&seen, 2).await;
        gate.notify_one();
        let (read, _, ended) = follow(&mut first).await;
        assert_eq!(ended, Ok(Outcome::Completed));
        assert_eq!(read.last(), Some(&id));
        assert_eq!(person_said(&seen, 1), ["one", "two"]);
        let turns = kinds(&server, thread).await;
        assert_eq!(turns.iter().filter(|k| *k == "turn.started").count(), 2);
        assert_eq!(turns.iter().filter(|k| *k == "turn.ended").count(), 2);
    });
}

#[test]
fn the_queue_is_bounded_a_message_chooses_no_model_and_stop_gives_back_what_waited() {
    let dir = folder("steer-bounded");
    block_on!(async {
        let answers = vec![answer(Some("Slow."), None), answer(Some("Again."), None)];
        let (server, seen, gate) = gated(&dir, answers).await;
        let thread = start(&server, false).await;
        let mut first = stream_turn(&server, thread, "slow", None).await.unwrap();
        asked(&seen, 1).await;
        let mut ids = Vec::new();
        for n in 0..glave_harness::MAX_QUEUED {
            ids.push(queue(&server, thread, &format!("message {n}")).await);
        }
        let full = server.request(send(thread, "one too many")).await;
        assert_eq!(full.unwrap_err().code, code::QUEUE_FULL);
        let choosing = Request::TurnStart {
            thread,
            prompt: "with a model".into(),
            background: false,
            budget: None,
            model: Some("test-model".into()),
            permissions: None,
            effort: None,
            meta: glave_session::protocol::Meta::default(),
            unattended: false,
            files: Vec::new(),
        };
        assert_eq!(
            server.request(choosing).await.unwrap_err().code,
            code::INVALID_PARAMS
        );
        // Stopped while the model answers: what waited is given back, never read.
        let cancel = server.request(Request::TurnCancel { thread });
        let (cancelled, followed) = tokio::join!(cancel, follow(&mut first));
        assert!(matches!(cancelled, Ok(Reply::Done {})));
        let (read, unread, ended) = followed;
        assert_eq!(ended, Ok(Outcome::Cancelled));
        assert_eq!(unread, ids);
        assert!(read.iter().all(|id| !ids.contains(id)));
        // A message sent after Stop starts a turn that goes on from there.
        let mut next = stream_turn(&server, thread, "go on", None).await.unwrap();
        gate.notify_one();
        let (_, _, ended) = follow(&mut next).await;
        assert_eq!(ended, Ok(Outcome::Completed));
    });
}

#[test]
fn a_message_sent_as_the_turn_is_stopped_starts_the_next_turn() {
    let dir = folder("steer-after-stop");
    block_on!(async {
        let answers = vec![answer(Some("Slow."), None), answer(Some("Next."), None)];
        let (server, seen, gate) = gated(&dir, answers).await;
        let thread = start(&server, false).await;
        let mut first = stream_turn(&server, thread, "slow", None).await.unwrap();
        asked(&seen, 1).await;
        // Stop is pressed first; the message, sent before the turn has ended, waits for it.
        let stop = server.request(Request::TurnCancel { thread });
        let caller = Caller::default();
        let sent = server.call(&caller, send(thread, "and now this"));
        let (stopped, sent, followed) = tokio::join!(stop, sent, follow(&mut first));
        assert!(matches!(stopped, Ok(Reply::Done {})));
        assert_eq!(followed.2, Ok(Outcome::Cancelled));
        let Answered::Stream(mut next) = sent else {
            panic!("a turn of its own")
        };
        gate.notify_one();
        let (read, _, ended) = follow(&mut next).await;
        assert_eq!(ended, Ok(Outcome::Completed));
        assert_eq!(read.len(), 1);
        assert_eq!(
            person_said(&seen, 1).last().map(String::as_str),
            Some("and now this")
        );
    });
}
