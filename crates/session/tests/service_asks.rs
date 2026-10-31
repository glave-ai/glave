//! What a connected service asks the person, through the session API: the thread waits, the
//! event says who asks and what, any client answers with `service/answer`, and the turn goes on;
//! what they typed is in no event a client reads.

use std::sync::Arc;

use glave_event::{Body, CallId, Outcome, Seq, ServiceAnswer, ThreadId, decode};
use glave_ports::ToolSource;
use glave_ports::testing::{ASKING_TOOL, AskingTool};
use glave_session::protocol::{Notification, Permissions, Reply, Request, Status, code};
use glave_session::{Answering, TurnItem};
use serde_json::json;

use crate::support::*;

const TYPED: &str = "only the service reads this";

fn typed() -> ServiceAnswer {
    ServiceAnswer::Answered {
        values: json!({"note": TYPED}).as_object().unwrap().clone(),
    }
}

/// A server whose model calls the tool of `source`, unasked by its mode, then says "Filed."
async fn asked_of(dir: &std::path::Path, source: &Arc<AskingTool>, answering: Answering) -> Tested {
    let answers = vec![
        answer(None, Some((ASKING_TOOL, json!({})))),
        answer(Some("Filed."), None),
    ];
    let mut setup = Setup::asking(answers, answering);
    setup.settings.permissions = Permissions::FullAccess;
    setup.tools = vec![Arc::clone(source) as Arc<dyn ToolSource>];
    setup.build(dir).await.0
}

fn answering(thread: ThreadId, call: &CallId, asked: Seq, answer: ServiceAnswer) -> Request {
    Request::ServiceAnswer {
        thread,
        call: call.clone(),
        asked,
        answer,
    }
}

#[test]
fn a_client_answers_what_a_service_asks_and_the_turn_goes_on() {
    let dir = folder("service-asks-answered");
    block_on!(async {
        let source = AskingTool::asking(&["Which repository?"]);
        let server = asked_of(&dir, &source, Answering::Clients).await;
        let thread = start(&server, false).await;
        let mut stream = stream_turn(&server, thread, "file it", None).await.unwrap();
        let mut ended = None;
        let mut told = Vec::new();
        while let Some(item) = stream.next().await {
            match item {
                TurnItem::Note(note) => {
                    let Notification::Event { event, .. } = note.as_ref() else {
                        continue;
                    };
                    let decoded = decode(event.clone()).unwrap();
                    match decoded.body {
                        Body::ServiceAsked(asked) => {
                            assert_eq!(asked.ask, "GitHub asks you, not the model");
                            assert_eq!(asked.message, "Which repository?");
                            match server.request(Request::ThreadGet { thread }).await.unwrap() {
                                Reply::Thread { thread } => {
                                    assert_eq!(thread.status, Status::Waiting);
                                }
                                other => panic!("{other:?}"),
                            }
                            let at = decoded.header.seq;
                            let misfit = ServiceAnswer::Answered {
                                values: serde_json::Map::new(),
                            };
                            let refused = answering(thread, &asked.call_id, at, misfit);
                            let refused = server.request(refused).await.err().unwrap();
                            assert_eq!(refused.code, code::INVALID_PARAMS);
                            let reply = answering(thread, &asked.call_id, at, typed());
                            server.request(reply).await.unwrap();
                        }
                        Body::ServiceAnswered(answered) => told.push(answered.told),
                        _ => {}
                    }
                }
                TurnItem::Ended(result) => ended = Some(result),
                TurnItem::Missed(_) => {}
            }
        }
        assert!(matches!(ended, Some(Ok(turn)) if turn.outcome == Outcome::Completed));
        assert_eq!(told, ["You answered GitHub"]);
        let given = source.given();
        assert_eq!(given.len(), 2);
        assert_eq!(given[1].as_ref().unwrap().answer, typed());
        // What a client reads back holds nothing of what was typed.
        for event in events(&server, thread).await {
            assert!(!event.to_json().unwrap().contains(TYPED));
        }
        let stale = answering(
            thread,
            &CallId::new("call_99").unwrap(),
            Seq::new(1).unwrap(),
            ServiceAnswer::Declined,
        );
        assert_eq!(
            server.request(stale).await.err().map(|e| e.code),
            Some(code::NO_QUESTION)
        );
    });
}

#[test]
fn with_no_one_to_answer_it_is_declined_and_nothing_waits() {
    let dir = folder("service-asks-nobody");
    block_on!(async {
        let source = AskingTool::asking(&["Which repository?"]);
        let server = asked_of(&dir, &source, Answering::NoOne).await;
        let thread = start(&server, false).await;
        let reply = turn(&server, thread, "file it").await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
        assert_eq!(source.unasked(), 1);
        assert!(source.given().is_empty());
    });
}
