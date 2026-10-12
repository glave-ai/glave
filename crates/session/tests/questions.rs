//! The model's questions through the session API: offered only when a client can answer, the
//! turn waiting until one does with `question/answer`, and the answers reaching the model.

use glave_event::{Body, CallId, Outcome, Part, ThreadId, decode};
use glave_session::protocol::{Notification, Reply, Request, Status, Update, code};
use glave_session::{Answering, TurnItem};
use serde_json::json;

use crate::support::*;

fn asks() -> serde_json::Value {
    json!({"questions": [
        {"question": "Which database?", "options": [{"label": "Postgres"}, {"label": "SQLite"}]},
        {"question": "Anything else?"}
    ]})
}

/// The text of every tool result in the thread.
async fn results(server: &Tested, thread: ThreadId) -> Vec<String> {
    events(server, thread)
        .await
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::ToolCallCompleted(done) => done.output.into_iter().find_map(|part| match part {
                Part::Text { text } => Some(text),
                _ => None,
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn a_client_answers_the_models_questions_and_the_turn_goes_on() {
    let dir = folder("questions-answered");
    block_on!(async {
        let answers = vec![
            answer(None, Some(("question", asks()))),
            answer(Some("Postgres it is."), None),
        ];
        let server = server(&dir, answers, Answering::Clients).await;
        let thread = start(&server, false).await;
        let mut stream = stream_turn(&server, thread, "set up storage", None)
            .await
            .unwrap();
        let mut asked = Vec::new();
        let mut ended = None;
        while let Some(item) = stream.next().await {
            match item {
                TurnItem::Note(note) => {
                    if let Notification::Live {
                        update:
                            Update::Asked {
                                question,
                                questions,
                            },
                        ..
                    } = note.as_ref()
                    {
                        match server.request(Request::ThreadGet { thread }).await.unwrap() {
                            Reply::Thread { thread } => {
                                assert_eq!(thread.status, Status::Waiting);
                            }
                            other => panic!("{other:?}"),
                        }
                        asked.extend(questions.iter().map(|q| q.question.clone()));
                        let reply = Request::QuestionAnswer {
                            thread,
                            question: question.clone(),
                            answers: vec!["Postgres".into(), String::new()],
                        };
                        server.request(reply).await.unwrap();
                    }
                }
                TurnItem::Ended(result) => ended = Some(result),
                TurnItem::Missed(_) => {}
            }
        }
        assert_eq!(asked, ["Which database?", "Anything else?"]);
        assert!(matches!(ended, Some(Ok(turn)) if turn.outcome == Outcome::Completed));
        assert_eq!(
            results(&server, thread).await,
            ["The person answered:\n- Which database? → Postgres\n- Anything else? → (no answer)"]
        );
        let stale = Request::QuestionAnswer {
            thread,
            question: CallId::new("call_99").unwrap(),
            answers: vec![],
        };
        assert_eq!(
            server.request(stale).await.err().map(|e| e.code),
            Some(code::NO_QUESTION)
        );
    });
}

#[test]
fn cancelling_a_turn_waiting_for_answers_ends_it() {
    let dir = folder("questions-cancelled");
    block_on!(async {
        let answers = vec![answer(None, Some(("question", asks())))];
        let server = server(&dir, answers, Answering::Clients).await;
        let thread = start(&server, false).await;
        let mut stream = stream_turn(&server, thread, "set up storage", None)
            .await
            .unwrap();
        let mut ended = None;
        while let Some(item) = stream.next().await {
            match item {
                TurnItem::Note(note) => {
                    if let Notification::Live {
                        update: Update::Asked { .. },
                        ..
                    } = note.as_ref()
                    {
                        server
                            .request(Request::TurnCancel { thread })
                            .await
                            .unwrap();
                    }
                }
                TurnItem::Ended(result) => ended = Some(result),
                TurnItem::Missed(_) => {}
            }
        }
        assert!(matches!(ended, Some(Ok(turn)) if turn.outcome == Outcome::Cancelled));
    });
}

#[test]
fn with_no_one_to_answer_question_is_not_offered() {
    let dir = folder("questions-nobody");
    block_on!(async {
        let answers = vec![
            answer(None, Some(("question", asks()))),
            answer(Some("I chose Postgres."), None),
        ];
        let server = server(&dir, answers, Answering::NoOne).await;
        let thread = start(&server, false).await;
        let reply = turn(&server, thread, "set up storage").await.unwrap();
        assert!(matches!(reply, Reply::Turn { turn } if turn.outcome == Outcome::Completed));
        let said = results(&server, thread).await;
        assert!(
            said[0].starts_with("There is no tool named \"question\""),
            "{said:?}"
        );
    });
}
