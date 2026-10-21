//! A turn's stream is a watch of its thread from where the turn began, which ends with how
//! the turn ended: on every transport, one pushed past its backlog still sends every event the
//! turn recorded before its answer; over HTTP its events carry their `seq`; and a connection of
//! lines writes a thread's notifications once, a turn in a thread it watches by that watch.

use std::sync::Arc;

use glave_event::Part;
use glave_ports::Caller;
use glave_session::protocol::{Notification, Request, Update, code};
use glave_session::{Answering, Watched};
use serde_json::Value;
use tokio::sync::Notify;

use crate::support::following::*;
use crate::support::*;

#[test]
fn a_turns_stream_pushed_past_its_backlog_still_sends_every_event_before_its_answer() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("turn-behind-{kind:?}").to_lowercase();
            let dir = folder(&name);
            // Far more notifications than a stream's backlog holds: one per part.
            let mut reply = answer(Some("start"), None);
            reply.parts = (0..3_000)
                .map(|n| Part::Text {
                    text: format!("part {n} "),
                })
                .collect();
            let server = server(&dir, vec![reply], Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let opened = logged(&server, thread).await;
            // A watch here sees the turn end; the client reads nothing meanwhile.
            let caller = Caller::default();
            let mut ending = server
                .watch(
                    &caller,
                    thread,
                    Some(glave_event::Seq::new(last(&opened)).unwrap()),
                )
                .await
                .unwrap();
            let mut client = over.watcher().await;
            client.start_turn(thread, "go on").await;
            loop {
                let Some(Watched::Note(note)) = ending.next().await else {
                    panic!("the watch ended before the turn did");
                };
                if let Notification::Event { event, .. } = note.as_ref()
                    && event.kind.as_str() == "turn.ended"
                {
                    break;
                }
            }
            let log = logged(&server, thread).await;
            let mut seen = Followed::default();
            read_through(&mut client, thread, last(&log), &mut seen).await;
            assert_eq!(seen.events, log[opened.len()..], "{kind:?}");
            let Said::Answer(answer) = client.next().await else {
                panic!("{kind:?}: the turn's answer comes after its last event");
            };
            assert_eq!(answer["result"]["turn"]["outcome"], "completed", "{kind:?}");
            over.close().await;
        }
    });
}

#[test]
fn a_turns_events_over_http_carry_their_seq_as_their_id() {
    block_on!(async {
        let name = "turn-ids";
        let dir = folder(name);
        let answers = vec![answer(Some("Hello there."), None)];
        let server = server(&dir, answers, Answering::Clients).await;
        let over = Over::new(Kind::Http, &server, name).await;
        let thread = start(&server, false).await;
        let opened = logged(&server, thread).await;
        let mut web = over.web();
        let (message, _) = web.message(&turn_start(thread, "hi", None));
        let text = web
            .open(&message, Some("turn/start"))
            .await
            .text()
            .await
            .unwrap();
        let log = logged(&server, thread).await;
        for recorded in &log[opened.len()..] {
            let seq = recorded.split(' ').next().unwrap();
            assert!(
                text.contains(&format!("id: {seq}\ndata: ")),
                "{seq}: {text}"
            );
        }
        assert!(
            text.contains("\n\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"turn/live\""),
            "a live update has no id: {text}"
        );
        over.close().await;
    });
}

/// What a connection of lines was sent until the turn answered request `turn`: its thread's
/// events (`seq`s), its live text, the turn's answer, and the other answers meanwhile.
struct UntilAnswered {
    seqs: Vec<u64>,
    texts: Vec<String>,
    answer: Value,
    others: Vec<Value>,
}

async fn until_answered(lines: &mut LinesWatcher, turn: u64) -> UntilAnswered {
    let (mut seqs, mut texts, mut others) = (Vec::new(), Vec::new(), Vec::new());
    loop {
        let message = lines.receive().await;
        if message["id"] == turn {
            let answer = message;
            return UntilAnswered {
                seqs,
                texts,
                answer,
                others,
            };
        }
        if message.get("method").is_none() {
            others.push(message);
            continue;
        }
        let Ok(note) = serde_json::from_value::<Notification>(message) else {
            continue;
        };
        match note {
            Notification::Event { event, .. } => seqs.push(event.seq.get()),
            Notification::Live {
                update: Update::Text { text },
                ..
            } => texts.push(text),
            _ => {}
        }
    }
}

#[test]
fn a_turn_in_a_thread_the_connection_watches_writes_each_notification_once_by_the_watch() {
    block_on!(async {
        for kind in [Kind::Stdio, Kind::Socket] {
            let name = format!("turn-watched-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let answers = vec![answer(Some("Hello there."), None)];
            let server = server(&dir, answers, Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let opened = logged(&server, thread).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, Some(last(&opened))).await;
            let Watcher::Lines(mut lines) = watcher else {
                unreachable!()
            };
            let turn = lines.send(&turn_start(thread, "hi", None)).await;
            let UntilAnswered {
                seqs,
                texts,
                answer: answered,
                ..
            } = until_answered(&mut lines, turn).await;
            let log = logged(&server, thread).await;
            let want: Vec<u64> = (last(&opened) + 1..=last(&log)).collect();
            assert_eq!(
                seqs, want,
                "{kind:?}: each event once, in order, before the answer"
            );
            assert_eq!(texts, ["Hello there."], "{kind:?}");
            assert_eq!(
                answered["result"]["turn"]["outcome"], "completed",
                "{kind:?}"
            );
            over.close().await;
        }
    });
}

#[test]
fn a_thread_whose_turn_the_connection_follows_is_watched_once_the_turn_has_answered() {
    block_on!(async {
        for kind in [Kind::Stdio, Kind::Socket] {
            let name = format!("turn-first-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let gate = Arc::new(Notify::new());
            let mut setup = Setup::new(vec![answer(Some("Finished."), None)]);
            setup.gate = Some(Arc::clone(&gate));
            let server = setup.build(&dir).await.0;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let Watcher::Lines(mut lines) = over.watcher().await else {
                unreachable!()
            };
            let turn = lines.send(&turn_start(thread, "go", None)).await;
            let watch = Request::ThreadWatch {
                thread,
                after: None,
            };
            let early = lines.send(&watch).await;
            let refused = loop {
                let message = lines.receive().await;
                if message["id"] == early {
                    break message;
                }
            };
            assert_eq!(refused["error"]["code"], code::INVALID_PARAMS, "{kind:?}");
            gate.notify_one();
            let answered = until_answered(&mut lines, turn).await.answer;
            assert_eq!(
                answered["result"]["turn"]["outcome"], "completed",
                "{kind:?}"
            );
            lines.send(&watch).await;
            loop {
                let message = lines.receive().await;
                if message["method"] == "thread/event" && message["params"]["event"]["seq"] == 1 {
                    break;
                }
                assert!(message.get("error").is_none(), "{kind:?}: {message}");
            }
            over.close().await;
        }
    });
}

#[test]
fn a_watch_unwatched_while_it_writes_a_turn_ends_once_the_turn_has_answered() {
    block_on!(async {
        for kind in [Kind::Stdio, Kind::Socket] {
            let name = format!("turn-unwatched-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let gate = Arc::new(Notify::new());
            let mut setup = Setup::new(vec![answer(Some("Finished."), None)]);
            setup.gate = Some(Arc::clone(&gate));
            let server = setup.build(&dir).await.0;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let opened = logged(&server, thread).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, Some(last(&opened))).await;
            let Watcher::Lines(mut lines) = watcher else {
                unreachable!()
            };
            let watch_id = lines.watching.unwrap();
            let turn = lines.send(&turn_start(thread, "go", None)).await;
            let unwatched = lines.send(&Request::ThreadUnwatch { thread }).await;
            gate.notify_one();
            // The turn's events and answer still come, by the watch, which then ends.
            let UntilAnswered {
                seqs,
                texts,
                answer: answered,
                others,
            } = until_answered(&mut lines, turn).await;
            let log = logged(&server, thread).await;
            let want: Vec<u64> = (last(&opened) + 1..=last(&log)).collect();
            assert_eq!(seqs, want, "{kind:?}");
            assert_eq!(texts, ["Finished."], "{kind:?}");
            assert_eq!(
                answered["result"]["turn"]["outcome"], "completed",
                "{kind:?}"
            );
            let mut answers = others;
            while !answers.iter().any(|answer| answer["id"] == watch_id) {
                answers.push(lines.receive().await);
            }
            let answer_to = |id: u64| answers.iter().find(|answer| answer["id"] == id);
            let ended = serde_json::json!({"ended": "unwatched"});
            assert_eq!(
                answer_to(watch_id).map(|a| &a["result"]),
                Some(&ended),
                "{kind:?}"
            );
            assert_eq!(
                answer_to(unwatched).map(|a| &a["result"]),
                Some(&serde_json::json!({})),
                "{kind:?}"
            );
            over.close().await;
        }
    });
}
