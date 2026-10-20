//! The Rust client of a session server elsewhere (`glave_session::client`), against the real
//! HTTP transport and a connection of lines, as on the person's Unix socket: requests
//! answered, a turn followed as the same items as in-process, an approval asked in the stream
//! and answered with a second request while it waits, and a wrong token refused saying how to
//! sign in. A client that leaves a connection of lines mid-turn stops nothing.

use std::sync::Arc;

use glave_event::{Body, Outcome, decode};
use glave_session::client::Remote;
use glave_session::protocol::{Answer, Meta, Notification, Reply, Request, TurnResult, code};
use glave_session::signin::{LocalToken, SignIn};
use glave_session::{Answering, TurnItem};
use serde_json::json;
use tokio::io::BufReader;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::support::issuer::fixed_clock;
use crate::support::web::serve_with;
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
        approval: None,
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
        approval: None,
        effort: None,
        meta: Meta::default(),
        unattended: false,
        files: Vec::new(),
    }
}

/// Follows a turn through `remote` to its end, allowing each approval it asks with a second
/// request while it waits; how it ended, and how many asked.
async fn follow_allowing(remote: &Remote, thread: glave_event::ThreadId) -> (TurnResult, usize) {
    let mut turn = remote
        .start(&turn_saying(thread, "touch it"))
        .await
        .unwrap();
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
fn a_client_that_leaves_mid_turn_stops_nothing_and_the_turn_finishes() {
    block_on!(async {
        let dir = folder("remote-leaves");
        let gate = Arc::new(Notify::new());
        let mut setup = Setup::new(vec![answer(Some("Finished anyway."), None)]);
        setup.gate = Some(Arc::clone(&gate));
        let server = setup.build(&dir).await.0;
        let (remote, serving) = over_lines(&server);
        let thread = thread_through(&remote).await;
        let mut turn = remote.start(&turn_saying(thread, "go")).await.unwrap();
        assert!(matches!(turn.next().await, Some(TurnItem::Note(_))));
        let mut watched = server.watch(thread);
        drop(turn);
        drop(remote);
        gate.notify_one();
        let ended = loop {
            let note = watched.recv().await.unwrap();
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
