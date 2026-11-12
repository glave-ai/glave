//! The end of a terminal's input answers nothing (`specs/one-server.md`, "The CLI as a
//! client"). It comes when the person types Ctrl-D, or when the terminal closes as its client is
//! killed, which on Linux the client can read before the hangup ends it. Either way the client
//! sends nothing, says so and stops, and what was asked waits for any client: an approval, the
//! model's questions, what a service asks, and a project's trust, which is asked again.
//! Each test types its answers and then the end of input (Ctrl-D at the start of a line) at
//! once: the input is read in order, so the end comes at the ask after the last answer.

use glave_event::{Body, ServiceAnswer, decode};
use glave_ports::testing::{HUNG, Running};
use glave_provider_openai::fake::{FakeServer, Reply};
use glave_session::protocol::{Request, Status};
use serde_json::json;

use crate::mcp::{settings, test_server};
use crate::support::*;
use crate::turns::{
    allowed_elsewhere, api, asking, events, newest_thread, once, recorded, status, touching,
    with_log,
};

/// What the terminal `turn` showed, once its input ended at what it asked: it stopped, saying
/// nothing was answered.
fn left(place: &Place, turn: Running) -> String {
    let said = turn.wait_with_output().unwrap();
    let shown = with_log(place, &text(&said));
    assert_eq!(said.status.code(), Some(1), "{shown}");
    assert!(shown.contains("nothing was answered"), "{shown}");
    shown
}

#[test]
fn an_approval_waits_for_any_client() {
    let place = asking("ua");
    let _stopped = Stopped(&place);
    let model = touching("Finished.", &[]);
    let mut turn = terminal(&place, &model, &["touch it"]);
    // A project here, then the end of input at the approval.
    typed(&mut turn, "y\n\x04");
    let shown = left(&place, turn);
    assert!(shown.contains("allow? [y] once"), "asked: {shown}");
    glave_ports::block_on!(async {
        let remote = api(&place);
        let thread = newest_thread(&remote, &place).await;
        let kinds = recorded(&remote, thread).await;
        assert!(
            !kinds.iter().any(|kind| kind == "approval.decided"),
            "{kinds:?}"
        );
        assert_eq!(status(&remote, thread).await, Status::Waiting, "{shown}");
        assert_eq!(model.received().len(), 1, "went no further: {shown}");
        allowed_elsewhere(&place, &model, &remote, thread).await;
    });
}

#[test]
fn the_model_s_questions_wait_for_any_client() {
    let place = asking("uq");
    let _stopped = Stopped(&place);
    let asks = json!({"questions": [{"question": "Which database?"}]});
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "question", &asks),
        Reply::text("Postgres it is."),
    ])
    .unwrap();
    let mut turn = terminal(&place, &model, &["set up storage"]);
    // A project here, then the end of input at the question.
    typed(&mut turn, "y\n\x04");
    let shown = left(&place, turn);
    assert!(shown.contains("Which database?"), "asked: {shown}");
    glave_ports::block_on!(async {
        let remote = api(&place);
        let thread = newest_thread(&remote, &place).await;
        assert_eq!(status(&remote, thread).await, Status::Waiting, "{shown}");
        assert_eq!(model.received().len(), 1, "went no further: {shown}");
        let question = events(&remote, thread)
            .await
            .into_iter()
            .find_map(|raw| match decode(raw).unwrap().body {
                Body::ToolCallRequested(call) if call.tool == "question" => Some(call.call_id),
                _ => None,
            })
            .unwrap();
        let answered = Request::QuestionAnswer {
            thread,
            question,
            answers: vec!["Postgres".to_owned()],
        };
        remote.request(&answered).await.unwrap();
        assert!(model.wait_for_requests(2, HUNG), "the turn went on");
        once(&place, &remote, thread, Status::Idle).await;
    });
}

#[test]
fn what_a_service_asks_waits_for_any_client() {
    // The end of input at a form's first field, and at whether to send a form with nothing to
    // fill in.
    for (name, what) in [("uf", "form"), ("un", "nothing")] {
        let place = asking(name);
        let _stopped = Stopped(&place);
        settings(&place, &json!({ "tests": { "command": test_server() } }));
        let model = FakeServer::start(vec![
            Reply::tool_call("call_1", "mcp__tests__ask", &json!({ "what": what })),
            Reply::text("It heard you."),
        ])
        .unwrap();
        let mut turn = terminal(&place, &model, &["file it"]);
        // A project here, then the call allowed, then the end of input at what it asks.
        typed(&mut turn, "y\ny\n\x04");
        let shown = left(&place, turn);
        assert!(shown.contains("tests asks you"), "asked: {shown}");
        let confirming = shown.contains("Send to tests? [y/N]");
        assert_eq!(confirming, what == "nothing", "{shown}");
        glave_ports::block_on!(async {
            let remote = api(&place);
            let thread = newest_thread(&remote, &place).await;
            let kinds = recorded(&remote, thread).await;
            assert!(
                !kinds.iter().any(|kind| kind == "service.answered"),
                "{kinds:?}"
            );
            assert_eq!(status(&remote, thread).await, Status::Waiting, "{shown}");
            assert_eq!(model.received().len(), 1, "went no further: {shown}");
            let (asked, call) = events(&remote, thread)
                .await
                .into_iter()
                .map(|raw| decode(raw).unwrap())
                .find_map(|event| match event.body {
                    Body::ServiceAsked(asked) => Some((event.header.seq, asked.call_id)),
                    _ => None,
                })
                .unwrap();
            let answered = Request::ServiceAnswer {
                thread,
                call,
                asked,
                answer: ServiceAnswer::Declined,
            };
            remote.request(&answered).await.unwrap();
            assert!(model.wait_for_requests(2, HUNG), "the turn went on");
            once(&place, &remote, thread, Status::Idle).await;
        });
    }
}

#[test]
fn a_project_s_trust_is_asked_again() {
    let place = asking("ut");
    let _stopped = Stopped(&place);
    crate::project::bring_server(&place);
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let mut first = terminal(&place, &model, &["hello"]);
    // A project here, then the end of input at whether to trust it.
    typed(&mut first, "y\n\x04");
    let shown = left(&place, first);
    assert!(shown.contains("Trust them?"), "asked: {shown}");
    assert!(model.received().is_empty(), "nothing ran: {shown}");
    glave_ports::block_on!(async {
        let remote = api(&place);
        let thread = newest_thread(&remote, &place).await;
        assert_eq!(status(&remote, thread).await, Status::Waiting, "{shown}");
    });
    // The same thread asks again, as it would not after "not now".
    let mut again = terminal(&place, &model, &["-c", "hello"]);
    typed(&mut again, "y\n");
    let said = again.wait_with_output().unwrap();
    let shown = with_log(&place, &text(&said));
    assert!(said.status.success(), "{shown}");
    assert!(shown.contains("Trust them?"), "asked again: {shown}");
    assert!(shown.contains("Hi."), "{shown}");
}
