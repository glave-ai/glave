//! Watching a thread (`thread/watch`) over every transport: in-process, a stream of lines as
//! stdio carries it, a Unix socket to a long-running server, and HTTP with server-sent events.
//! Rebuilding from `after` and then watching equals watching from the start; a watcher pushed
//! past its backlog still gets every event; one that leaves and comes back with `after` misses
//! nothing; and a watch ends when its thread is deleted, or when the client ends it.

use glave_event::{Body, Part, decode};
use glave_ports::Caller;
use glave_session::Answering;
use glave_session::protocol::{Notification, Request, code};
use serde_json::{Value, json};

use crate::support::following::*;
use crate::support::*;

#[test]
fn rebuilding_from_after_then_watching_equals_watching_from_the_start() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("rebuild-{kind:?}").to_lowercase();
            let (server, _dir) = scripted(&name).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let (mut from_start, mut from_after) = (Followed::default(), Followed::default());
            let mut first = over.watcher().await;
            first.watch(thread, None).await;
            // Watching once what was recorded before has come: a request over a connection is
            // answered in its own time.
            let opened = logged(&server, thread).await;
            read_through(&mut first, thread, last(&opened), &mut from_start).await;
            run(&server, thread, "hi").await;
            let before = logged(&server, thread).await;
            let k = u64::try_from(before.len() / 2).unwrap();
            let mut second = over.watcher().await;
            second.watch(thread, Some(k)).await;
            read_through(&mut second, thread, last(&before), &mut from_after).await;
            run(&server, thread, "touch it").await;
            let log = logged(&server, thread).await;
            assert!(log.iter().any(|kind| kind.ends_with("approval.decided")));
            read_through(&mut first, thread, last(&log), &mut from_start).await;
            read_through(&mut second, thread, last(&log), &mut from_after).await;
            assert_eq!(from_start.events, log, "{kind:?}");
            let rebuilt: Vec<String> = log[..usize::try_from(k).unwrap()]
                .iter()
                .chain(&from_after.events)
                .cloned()
                .collect();
            assert_eq!(rebuilt, log, "{kind:?}");
            // Both watched the second turn as it ran: the same events and live updates.
            let second_turn = |seen: &Followed| {
                let started = seen
                    .all
                    .iter()
                    .rposition(|shown| shown.ends_with("turn.started"))
                    .unwrap();
                seen.all[started..].to_vec()
            };
            assert_eq!(
                second_turn(&from_start),
                second_turn(&from_after),
                "{kind:?}"
            );
            over.close().await;
        }
    });
}

#[test]
fn a_watcher_pushed_past_its_backlog_still_gets_every_event() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("behind-{kind:?}").to_lowercase();
            let dir = folder(&name);
            // Far more notifications than a watcher's backlog holds: one per part.
            let mut reply = answer(Some("start"), None);
            reply.parts = (0..3_000)
                .map(|n| Part::Text {
                    text: format!("part {n} "),
                })
                .collect();
            let server = server(&dir, vec![reply], Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, None).await;
            let mut seen = Followed::default();
            let opened = logged(&server, thread).await;
            read_through(&mut watcher, thread, last(&opened), &mut seen).await;
            // The watcher reads nothing while the turn runs to its end.
            run(&server, thread, "go on").await;
            let log = logged(&server, thread).await;
            read_through(&mut watcher, thread, last(&log), &mut seen).await;
            assert_eq!(seen.events, log, "{kind:?}");
            over.close().await;
        }
    });
}

#[test]
fn a_watcher_that_comes_back_with_after_misses_nothing() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("again-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let gate = std::sync::Arc::new(tokio::sync::Notify::new());
            let mut setup = Setup::new(vec![answer(Some("Finished."), None)]);
            setup.gate = Some(std::sync::Arc::clone(&gate));
            let server = setup.build(&dir).await.0;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, None).await;
            let mut seen = Followed::default();
            let opened = logged(&server, thread).await;
            read_through(&mut watcher, thread, last(&opened), &mut seen).await;
            let mut stream = stream_turn(&server, thread, "go", None).await.unwrap();
            // The turn has started and taken the message; its answer waits at the gate.
            let had = read_until(&mut watcher, thread, &mut seen, |_, kind| {
                kind == "message.added"
            })
            .await;
            watcher.leave(thread).await;
            gate.notify_one();
            stream.finish().await.unwrap();
            let log = logged(&server, thread).await;
            // Back where it left off: the same connection over stdio, whose closing would stop
            // the server; a new one elsewhere.
            if kind != Kind::Stdio {
                watcher = over.watcher().await;
            }
            watcher.watch(thread, Some(had)).await;
            read_through(&mut watcher, thread, last(&log), &mut seen).await;
            assert_eq!(seen.events, log, "{kind:?}");
            over.close().await;
        }
    });
}

#[test]
fn deleting_the_thread_ends_its_watch() {
    block_on!(async {
        for kind in KINDS {
            let name = format!("deleted-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let thread = start(&server, false).await;
            let mut watcher = over.watcher().await;
            watcher.watch(thread, None).await;
            // Watching once what was recorded has come.
            let mut seen = Followed::default();
            let opened = logged(&server, thread).await;
            read_through(&mut watcher, thread, last(&opened), &mut seen).await;
            server
                .request(Request::ThreadDelete { thread })
                .await
                .unwrap();
            let ended = loop {
                match watcher.next().await {
                    Said::Note(Notification::Event { event, .. }) => {
                        let kind = decode(event).unwrap().body;
                        if let Body::ThreadDeleted(_) = kind {
                            let Said::Answer(ended) = watcher.next().await else {
                                panic!("the watch went on after its thread was deleted");
                            };
                            break ended;
                        }
                    }
                    Said::Note(_) => {}
                    Said::Answer(answer) => panic!("ended before the deletion: {answer}"),
                }
            };
            assert_eq!(ended["result"], json!({"ended": "deleted"}), "{kind:?}");
            // A deleted thread is not there to watch.
            let refused = server
                .watch(&Caller::default(), thread, None)
                .await
                .err()
                .unwrap();
            assert_eq!(refused.code, code::NO_THREAD);
            over.close().await;
        }
    });
}

#[test]
fn a_connection_of_lines_watches_each_thread_once_and_at_most_64() {
    block_on!(async {
        for kind in [Kind::Stdio, Kind::Socket] {
            let name = format!("most-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let Watcher::Lines(mut lines) = over.watcher().await else {
                unreachable!()
            };
            let mut threads = Vec::new();
            for _ in 0..65 {
                threads.push(start(&server, false).await);
            }
            for thread in &threads[..64] {
                lines
                    .send(&Request::ThreadWatch {
                        thread: *thread,
                        after: None,
                    })
                    .await;
            }
            // Each watch is held once it sends its thread's first event.
            let mut heard = std::collections::BTreeSet::new();
            while heard.len() < 64 {
                let message = lines.receive().await;
                if message["method"] == "thread/event" {
                    heard.insert(message["params"]["thread"].as_str().unwrap().to_owned());
                }
            }
            let refused = |message: &Value| message["error"]["code"].as_i64();
            let again = lines
                .send(&Request::ThreadWatch {
                    thread: threads[0],
                    after: None,
                })
                .await;
            let over_the_top = lines
                .send(&Request::ThreadWatch {
                    thread: threads[64],
                    after: None,
                })
                .await;
            let (mut twice, mut most) = (None, None);
            while twice.is_none() || most.is_none() {
                let message = lines.receive().await;
                if message["id"] == again {
                    twice = refused(&message);
                } else if message["id"] == over_the_top {
                    most = refused(&message);
                }
            }
            assert_eq!(twice, Some(code::INVALID_PARAMS), "{kind:?}");
            assert_eq!(most, Some(code::LIMIT), "{kind:?}");
            over.close().await;
        }
    });
}

#[test]
fn a_connection_of_lines_takes_watches_and_unwatches_in_the_order_sent() {
    block_on!(async {
        for kind in [Kind::Stdio, Kind::Socket] {
            let name = format!("order-{kind:?}").to_lowercase();
            let dir = folder(&name);
            let server = server(&dir, Vec::new(), Answering::Clients).await;
            let mut over = Over::new(kind, &server, &name).await;
            let freed = start(&server, false).await;
            let mut watcher = over.watcher().await;
            watcher.watch(freed, None).await;
            let mut seen = Followed::default();
            read_through(&mut watcher, freed, 1, &mut seen).await;
            let Watcher::Lines(mut lines) = watcher else {
                unreachable!()
            };
            // Each in the order sent: an unwatch ends the watch sent before it, and a watch sent
            // right after it is not refused.
            let watch = Request::ThreadWatch {
                thread: freed,
                after: None,
            };
            let unwatch = Request::ThreadUnwatch { thread: freed };
            let mut sent = Vec::new();
            for request in [&unwatch, &watch, &unwatch, &watch] {
                sent.push(lines.send(request).await);
            }
            let mut answers = std::collections::BTreeMap::new();
            while answers.len() < 5 {
                let message = lines.receive().await;
                if let Some(id) = message.get("id").and_then(Value::as_u64)
                    && sent.contains(&id)
                {
                    answers.insert(id, message);
                }
                // The last watch runs until it, too, is unwatched.
                if answers.len() == 3 && sent.len() == 4 {
                    sent.push(lines.send(&unwatch).await);
                }
            }
            let ended = json!({"ended": "unwatched"});
            for (at, answer) in [json!({}), ended.clone(), json!({}), ended, json!({})]
                .into_iter()
                .enumerate()
            {
                assert_eq!(answers[&sent[at]]["result"], answer, "{kind:?} {at}");
            }
            over.close().await;
        }
    });
}
