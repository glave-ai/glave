//! The CLI as a client of a session server elsewhere: `glave serve --http` in one home, and
//! `glave --server` from another with nothing of its own, signed in with the server's token.
//! The turn runs there and shows here, `--continue` continues it there, and nothing is kept
//! on the client's side. Without a token, it says how to sign in; a model it names goes to
//! the server, which refuses one it does not run. An approval is answered once it waits, never
//! when the judge already let its action run.

use std::process::{Command, Stdio};

use glave_ports::testing::Running;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::http::serve_http;
use crate::support::*;

/// `glave` in a home of its own with `args`, as a client of the server at `url`.
fn client(place: &Place, args: &[&str], token: Option<&str>) -> std::process::Output {
    let home = place.project.join("../client-home");
    let mut command = Command::new(env!("CARGO_BIN_EXE_glave"));
    command
        .args(args)
        .current_dir(&place.project)
        .env("GLAVE_HOME", &home)
        .env("NO_COLOR", "1")
        .env_remove("GLAVE_BASE_URL")
        .env_remove("GLAVE_API_KEY")
        .env_remove("GLAVE_MODEL")
        .stdin(Stdio::null());
    match token {
        Some(token) => command.env("GLAVE_SERVER_TOKEN", token),
        None => command.env_remove("GLAVE_SERVER_TOKEN"),
    };
    command.output().unwrap()
}

#[test]
fn a_turn_runs_on_the_server_and_shows_here() {
    let place = place("remote");
    let model = FakeServer::start(vec![
        Reply::text("Answered over there."),
        Reply::text("Still there."),
    ])
    .unwrap();
    let (_served, url) = serve_http(&place, &model, &["--model", "fake-model"]);
    let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();

    let first = client(&place, &["--server", &url, "where are you?"], Some(&token));
    let shown = text(&first);
    assert!(first.status.success(), "{shown}");
    assert!(shown.contains("Answered over there."), "{shown}");
    assert!(shown.contains("done"), "{shown}");
    let again = client(
        &place,
        &["--server", &url, "--continue", "and now?"],
        Some(&token),
    );
    let shown = text(&again);
    assert!(again.status.success(), "{shown}");
    assert!(shown.contains("Still there."), "{shown}");
    let received = model.received();
    assert_eq!(received.len(), 2);
    let second = said(&received[1].body, "user");
    assert!(
        second.iter().any(|text| text.contains("where are you?")),
        "the second turn continues the first thread: {second:?}"
    );
    let client_home = place.project.join("../client-home");
    assert!(!client_home.join("glave.db").exists(), "nothing kept here");

    let unsigned = client(&place, &["--server", &url, "hi"], None);
    assert!(!unsigned.status.success());
    assert!(
        text(&unsigned).contains("GLAVE_SERVER_TOKEN"),
        "{}",
        text(&unsigned)
    );
    // A model goes to the server, which refuses one it does not run.
    let unknown = client(
        &place,
        &["--server", &url, "--model", "m", "hi"],
        Some(&token),
    );
    assert!(!unknown.status.success());
    assert!(text(&unknown).contains("model/list"), "{}", text(&unknown));
}

/// `glave serve --http` where commands run as the person and only cautions ask, the judge
/// first, with `model`'s replies, stopped when dropped; its address and token.
fn judging(place: &Place, model: &FakeServer) -> (Running, String, String) {
    let args = [
        "--model",
        "fake-model",
        "--sandbox",
        "off",
        "--approval",
        "auto",
    ];
    let (served, url) = serve_http(place, model, &args);
    let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
    (served, url, token)
}

#[test]
fn an_action_the_judge_let_run_is_not_answered_again() {
    let place = place("remote-judged");
    let push = "git push --dry-run origin main";
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "bash", &json!({ "command": push })),
        Reply::text(r#"{"decision":"run","reason":"the person asked to push"}"#),
        Reply::text("Pushed."),
    ])
    .unwrap();
    let (_served, url, token) = judging(&place, &model);

    let pushed = client(&place, &["--server", &url, "push my branch"], Some(&token));
    let shown = text(&pushed);
    assert!(pushed.status.success(), "{shown}");
    // Requested and decided at once, so nothing was left to answer.
    assert!(!shown.contains("no approval"), "{shown}");
    assert!(!shown.contains("declined"), "{shown}");
    assert!(shown.contains(&format!("bash: {push} …")), "{shown}");
    assert!(shown.contains("Pushed."), "{shown}");
    let received = model.received();
    assert_eq!(received.len(), 3, "the turn's two calls and the judge's");
    let result = said(&received[2].body, "tool").concat();
    assert!(
        !result.contains("declined") && !result.contains("Not run"),
        "it ran: {result}"
    );
}

#[test]
fn an_action_the_judge_leaves_to_the_person_is_still_answered() {
    let place = place("remote-unjudged");
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "bash", &json!({ "command": "npm publish" })),
        Reply::text(r#"{"decision":"ask","reason":"asked to bump, not to publish"}"#),
        Reply::text("Left it."),
    ])
    .unwrap();
    let (_served, url, token) = judging(&place, &model);

    let bumped = client(
        &place,
        &["--server", &url, "bump the version"],
        Some(&token),
    );
    let shown = text(&bumped);
    assert!(bumped.status.success(), "{shown}");
    // It waits, and with no one at this terminal the answer is no.
    assert!(shown.contains("bash: npm publish … declined"), "{shown}");
    assert!(!shown.contains("no approval"), "{shown}");
    assert!(shown.contains("Left it."), "{shown}");
}
