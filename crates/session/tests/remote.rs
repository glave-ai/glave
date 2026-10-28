//! The Rust client of a session server elsewhere (`glave_session::client`), against the real
//! HTTP transport and a connection of lines, as on the person's Unix socket: requests
//! answered, a turn followed as the same items as in-process, an approval asked in the stream
//! and answered with a second request while it waits, a message sent while a turn runs answered
//! as queued, a page of a thread's events read as in-process, and a wrong token refused saying
//! how to sign in. A client that leaves a connection of lines mid-turn stops nothing.

use std::sync::Arc;

use glave_event::{Body, Outcome, decode};
use glave_ports::Caller;
use glave_session::client::Remote;
use glave_session::protocol::{Answer, Meta, Notification, Reply, Request, TurnResult, code};
use glave_session::signin::{LocalToken, SignIn};
use glave_session::{Answering, Sent, TurnItem, Watched};
use serde_json::json;
use tokio::io::BufReader;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::support::issuer::fixed_clock;
use crate::support::web::{Served, serve_with};
use crate::support::*;

/// The answers of a turn that asks once to run a command, then is done.
fn asking_once() -> Vec<glave_ports::Completion> {
    vec![
        answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
        answer(Some("Done."), None),
    ]
}

/// A new thread in the test's project, started through `remote`.
async fn thread_through(remote: &Remote) -> glave_event::ThreadId {
    let started = Request::ThreadStart {
        project: Some(scope().project()),
        worktree: None,
        background: false,
        computer: None,
        model: None,
        permissions: None,
        effort: None,
    };
    let Reply::Thread { thread } = remote.request(&started).await.unwrap() else {
        panic!("a thread")
    };
    thread.id
}

/// `turn/start` saying `prompt` in `thread`.
fn turn_saying(thread: glave_event::ThreadId, prompt: &str) -> Request {
    Request::TurnStart {
        thread,
        prompt: prompt.into(),
        background: false,
        budget: None,
        model: None,
        permissions: None,
        effort: None,
        meta: Meta::default(),
        unattended: false,
        files: Vec::new(),
    }
}

/// Follows a turn through `remote` to its end, allowing each approval it asks with a second
/// request while it waits; how it ended, and how many asked.
async fn follow_allowing(remote: &Remote, thread: glave_event::ThreadId) -> (TurnResult, usize) {
    let Ok(Sent::Started(mut turn)) = remote.start(&turn_saying(thread, "touch it")).await else {
        panic!("a turn of its own");
    };
    let mut asked = 0;
    let ended = loop {
        match turn.next().await.unwrap() {
            TurnItem::Ended(ended) => break ended,
            TurnItem::Note(note) => {
                let Notification::Event { thread, event } = note.as_ref() else {
                    continue;
                };
                if let Body::ApprovalRequested(request) = decode(event.clone()).unwrap().body {
                    asked += 1;
                    let decide = Request::ApprovalDecide {
                        thread: *thread,
                        approval: request.call_id,
                        decision: Answer::AllowOnce,
                    };
                    remote.request(&decide).await.unwrap();
                }
            }
            TurnItem::Missed(_) => {}
        }
    };
    assert!(turn.next().await.is_none(), "nothing after the ending");
    (ended.unwrap(), asked)
}

/// `server` served over a connection of lines as the person's server serves its socket, and a
/// client of it; the serving task, which ends when the client goes.
fn over_lines(server: &Tested) -> (Remote, JoinHandle<std::io::Result<()>>) {
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    let (read, write) = tokio::io::split(theirs);
    let serving = tokio::spawn(glave_session::rpc::serve_connection(
        server.clone(),
        BufReader::new(read),
        write,
    ));
    let (read, write) = tokio::io::split(ours);
    let remote = Remote::over_lines(BufReader::new(read), write, "glave".into());
    (remote, serving)
}

#[test]
fn a_turn_elsewhere_streams_and_its_approvals_are_answered_as_here() {
    block_on!(async {
        let dir = folder("remote-client");
        let token = LocalToken::new([9; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let served = serve_with(
            server(&dir, asking_once(), Answering::Clients).await,
            signin,
            None,
        )
        .await;
        let remote = Remote::new(&served.url, token.reveal().to_owned()).unwrap();

        let found = remote.request(&Request::Discover {}).await.unwrap();
        assert!(matches!(found, Reply::Server(_)), "{found:?}");
        let thread = thread_through(&remote).await;
        let (ended, asked) = follow_allowing(&remote, thread).await;
        assert_eq!(ended.outcome, Outcome::Completed);
        assert_eq!(asked, 1, "the command was asked about once");

        let wrong = Remote::new(&served.url, "glave_local_wrong".into()).unwrap();
        let refused = wrong.request(&Request::Discover {}).await.unwrap_err();
        assert_eq!(refused.code, code::NO_SIGN_IN);
        assert!(refused.message.contains("GLAVE_SERVER_TOKEN"), "{refused}");
        assert!(!refused.message.contains("glave_local_wrong"), "{refused}");
        served.close().await;
    });
}

/// `server` served over HTTP, signed in with a local token, and a client of it signed in.
async fn over_http(server: &Tested) -> (Remote, Served) {
    let token = LocalToken::new([9; 32]);
    let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
    let web = serve_with(server.clone(), signin, None).await;
    let http = Remote::new(&web.url, token.reveal().to_owned()).unwrap();
    (http, web)
}

#[test]
fn a_turn_over_a_connection_of_lines_is_followed_and_answered_as_over_http() {
    block_on!(async {
        let dir = folder("remote-lines");
        let server = server(&dir, asking_once(), Answering::Clients).await;
        let (remote, serving) = over_lines(&server);
        let thread = thread_through(&remote).await;
        let (ended, asked) = follow_allowing(&remote, thread).await;
        assert_eq!(ended.outcome, Outcome::Completed);
        assert_eq!(asked, 1, "the command was asked about once");
        assert!(dir.join("ran.txt").exists());
        // Refused before it starts: a second turn's choice of a model no one offers.
        let mut refused = turn_saying(thread, "again");
        if let Request::TurnStart { model, .. } = &mut refused {
            *model = Some("nobody's".into());
        }
        let refused = remote.start(&refused).await.err().map(|error| error.code);
        assert_eq!(refused, Some(code::NO_MODEL));
        drop(remote);
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn a_message_sent_while_a_turn_runs_is_answered_as_queued_over_http_and_lines() {
    block_on!(async {
        let dir = folder("remote-queued");
        let gate = Arc::new(Notify::new());
        let answers = vec![answer(Some("First."), None), answer(Some("Then."), None)];
        let mut setup = Setup::new(answers);
        setup.gate = Some(Arc::clone(&gate));
        let (server, seen) = setup.build(&dir).await;
        let (http, web) = over_http(&server).await;
        let (lines, serving) = over_lines(&server);
        let thread = thread_through(&http).await;
        let mut running = stream_turn(&server, thread, "one", None).await.unwrap();
        asked(&seen, 1).await;
        let mut ids = Vec::new();
        for remote in [&http, &lines] {
            match remote.start(&turn_saying(thread, "and this")).await {
                Ok(Sent::Queued(queued)) => ids.push(queued.id),
                Ok(Sent::Started(_)) => panic!("queued, not a turn of its own"),
                Err(error) => panic!("queued, not refused: {error}"),
            }
        }
        assert_ne!(ids.first(), ids.last(), "two messages, each its own id");
        // Too late for the turn's one call, they start the next.
        gate.notify_one();
        asked(&seen, 2).await;
        gate.notify_one();
        assert_eq!(running.finish().await.unwrap().outcome, Outcome::Completed);
        drop((http, lines));
        web.close().await;
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn a_client_that_leaves_mid_turn_stops_nothing_and_the_turn_finishes() {
    block_on!(async {
        let dir = folder("remote-leaves");
        let gate = Arc::new(Notify::new());
        let mut setup = Setup::new(vec![answer(Some("Finished anyway."), None)]);
        setup.gate = Some(Arc::clone(&gate));
        let server = setup.build(&dir).await.0;
        let (remote, serving) = over_lines(&server);
        let thread = thread_through(&remote).await;
        let Ok(Sent::Started(mut turn)) = remote.start(&turn_saying(thread, "go")).await else {
            panic!("a turn of its own");
        };
        assert!(matches!(turn.next().await, Some(TurnItem::Note(_))));
        let mut watched = server
            .watch(&Caller::default(), thread, None)
            .await
            .unwrap();
        drop(turn);
        drop(remote);
        gate.notify_one();
        let ended = loop {
            let Some(Watched::Note(note)) = watched.next().await else {
                panic!("the watch ended before the turn did")
            };
            if let Notification::Event { event, .. } = note.as_ref()
                && let Body::TurnEnded(ended) = decode(event.clone()).unwrap().body
            {
                break ended;
            }
        };
        assert_eq!(ended.outcome, Outcome::Completed);
        // The connection's requests end with the turn they followed; writing to the client
        // that left failed, which is all.
        let _written = serving.await.unwrap();
    });
}

/// `thread/events` through `remote`, `limit` events from the start, or all of them.
async fn page_through(
    remote: &Remote,
    thread: glave_event::ThreadId,
    limit: Option<usize>,
) -> serde_json::Value {
    let page = Request::ThreadEvents {
        thread,
        after: None,
        limit,
    };
    let read = remote.request(&page).await.unwrap();
    assert!(matches!(read, Reply::Events { .. }), "{read:?}");
    serde_json::to_value(read).unwrap()
}

#[test]
fn a_page_of_events_reads_through_the_client_as_in_process_over_http_and_lines() {
    block_on!(async {
        let dir = folder("remote-events");
        let server = server(&dir, asking_once(), Answering::NoOne).await;
        let (http, web) = over_http(&server).await;
        let (lines, serving) = over_lines(&server);
        let thread = thread_through(&http).await;
        let mut turn = stream_turn(&server, thread, "touch it", None)
            .await
            .unwrap();
        turn.finish().await.unwrap();
        for limit in [None, Some(2)] {
            let page = Request::ThreadEvents {
                thread,
                after: None,
                limit,
            };
            let here = serde_json::to_value(server.request(page).await.unwrap()).unwrap();
            // The whole page, and the first two with where the next page starts.
            let read = here["events"].as_array().unwrap().len();
            assert_eq!(read > 2, limit.is_none(), "{here}");
            assert_eq!(here["next"].is_null(), limit.is_none(), "{here}");
            for remote in [&http, &lines] {
                assert_eq!(
                    page_through(remote, thread, limit).await,
                    here,
                    "{}",
                    remote.url()
                );
            }
        }
        drop((http, lines));
        web.close().await;
        serving.await.unwrap().unwrap();
    });
}
