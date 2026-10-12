//! The Rust client of a session server elsewhere (`glave_session::client`), against the real
//! HTTP transport: requests answered, a turn followed as the same items as in-process, an
//! approval asked in the stream and answered with a second request while it waits, and a
//! wrong token refused saying how to sign in.

use glave_event::{Body, decode};
use glave_session::client::Remote;
use glave_session::protocol::{Answer, Meta, Notification, Reply, Request, code};
use glave_session::signin::{LocalToken, SignIn};
use glave_session::{Answering, TurnItem};
use serde_json::json;

use crate::support::issuer::fixed_clock;
use crate::support::web::serve_with;
use crate::support::*;

#[test]
fn a_turn_elsewhere_streams_and_its_approvals_are_answered_as_here() {
    block_on!(async {
        let dir = folder("remote-client");
        let answers = vec![
            answer(None, Some(("bash", json!({"command": "touch ran.txt"})))),
            answer(Some("Done."), None),
        ];
        let token = LocalToken::new([9; 32]);
        let signin = SignIn::local(token.clone(), Box::new(fixed_clock()));
        let served = serve_with(
            server(&dir, answers, Answering::Clients).await,
            signin,
            None,
        )
        .await;
        let remote = Remote::new(&served.url, token.reveal().to_owned()).unwrap();

        let found = remote.request(&Request::Discover {}).await.unwrap();
        assert!(matches!(found, Reply::Server(_)), "{found:?}");
        let started = Request::ThreadStart {
            space: None,
            worktree: None,
            background: false,
            computer: None,
            model: None,
            approval: None,
        };
        let Reply::Thread { thread } = remote.request(&started).await.unwrap() else {
            panic!("a thread")
        };
        let mut turn = remote
            .start(&Request::TurnStart {
                thread: thread.id,
                prompt: "touch it".into(),
                background: false,
                budget: None,
                model: None,
                approval: None,
                meta: Meta::default(),
            })
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
        let ended = ended.unwrap();
        assert_eq!(ended.outcome, glave_event::Outcome::Completed);
        assert_eq!(asked, 1, "the command was asked about once");
        assert!(turn.next().await.is_none(), "nothing after the ending");

        let wrong = Remote::new(&served.url, "glave_local_wrong".into()).unwrap();
        let refused = wrong.request(&Request::Discover {}).await.unwrap_err();
        assert_eq!(refused.code, code::NO_SIGN_IN);
        assert!(refused.message.contains("GLAVE_SERVER_TOKEN"), "{refused}");
        assert!(!refused.message.contains("glave_local_wrong"), "{refused}");
        served.close().await;
    });
}
