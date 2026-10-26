//! The person's settings read again, through the real binary over stdio with a fresh home
//! (`specs/one-server.md`, "Its settings"): an edited `config.toml` changes the next turn's
//! model, and a server added to `mcp.json` starts while a removed one stops, with no restart;
//! a changed mode holds from the next turn while the running one keeps its own; the
//! network turned off is off for the next turn's commands; and the judge follows the models.

use std::path::{Path, PathBuf};

use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::{Value, json};

use crate::served::*;
use crate::support::*;

/// The test server, built beside `glave` by `cargo xtask ci`.
fn test_server() -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_glave")).with_file_name("glave-mcp-test-server");
    assert!(
        path.exists(),
        "build the test server first: cargo build -p glave-mcp --features test-server"
    );
    path
}

/// The person's `config.toml` choosing `model`.
fn choosing(home: &Path, model: &str) {
    let config = format!("model = \"{model}\"\nmemory = \"off\"\ntitles = false\n");
    std::fs::write(home.join("config.toml"), config).unwrap();
}

/// One turn in a new chat.
fn turn(served: &mut Served) {
    let started = served.call("thread/start", &json!({}));
    let thread = started["result"]["thread"]["id"].clone();
    let ran = served.call("turn/start", &json!({"thread": thread, "prompt": "go"}));
    assert_eq!(ran["result"]["turn"]["outcome"], "completed", "{ran}");
}

/// The model the latest request went to, and whether it offered the person's server.
fn latest(model: &FakeServer) -> (String, bool) {
    let received = model.received();
    let body: &Value = &received.last().unwrap().body;
    let offered = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["function"]["name"] == "mcp__mine__echo");
    (body["model"].as_str().unwrap().to_owned(), offered)
}

/// The processes glave records as running, under `home`.
fn running(home: &Path) -> usize {
    std::fs::read_dir(home.join("run/processes")).map_or(0, |listed| {
        listed
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count()
    })
}

#[test]
fn an_edited_config_changes_the_next_turn_s_model_and_mcp_json_is_reconciled() {
    let place = place("settings-again");
    std::fs::create_dir_all(&place.home).unwrap();
    choosing(&place.home, "m");
    let answers = (0..3).map(|_| Reply::text("Done.")).collect();
    let model = FakeServer::start(answers).unwrap();
    let mut served = Served::start(&place, &model, &["--permissions", "full-access"]);
    turn(&mut served);
    assert_eq!(latest(&model), ("m".to_owned(), false));
    assert_eq!(running(&place.home), 0);

    choosing(&place.home, "n");
    let mine = json!({ "mcpServers": { "mine": { "command": test_server() } } });
    std::fs::write(place.home.join("mcp.json"), mine.to_string()).unwrap();
    assert_eq!(served.models(), ["n"], "the models listed follow too");
    turn(&mut served);
    assert_eq!(
        latest(&model),
        ("n".to_owned(), true),
        "the edit and the added server"
    );
    assert_eq!(running(&place.home), 1, "the added server runs");

    std::fs::remove_file(place.home.join("mcp.json")).unwrap();
    turn(&mut served);
    assert_eq!(
        latest(&model),
        ("n".to_owned(), false),
        "the removed server is gone"
    );
    assert_eq!(running(&place.home), 0, "and stopped");
    served.end();
}

/// The person's `config.toml` choosing `model`, with `more`.
fn writing(home: &Path, model: &str, more: &str) {
    let config = format!("model = \"{model}\"\nmemory = \"off\"\ntitles = false\n{more}");
    std::fs::write(home.join("config.toml"), config).unwrap();
}

/// A `bash` call of `command`, as the model makes it.
fn bash(id: &str, command: &str) -> Reply {
    Reply::tool_call(id, "bash", &json!({ "command": command }))
}

/// A turn of `prompt` in `thread`, each approval it waits for answered by `decide`; how
/// many asked.
fn turn_in(
    served: &mut Served,
    thread: &Value,
    decide: impl FnMut(&Value) -> &'static str,
) -> usize {
    let params = json!({"thread": thread, "prompt": "go"});
    let (ran, asked) = served.call_deciding("turn/start", &params, decide);
    assert_eq!(ran["result"]["turn"]["outcome"], "completed", "{ran}");
    asked
}

fn new_thread(served: &mut Served) -> Value {
    let started = served.call("thread/start", &json!({}));
    started["result"]["thread"]["id"].clone()
}

/// What the model was sent back from its latest tool call, in request `index`.
fn result(model: &FakeServer, index: usize) -> String {
    let said = said(&model.received()[index].body, "tool");
    said.last().cloned().unwrap_or_default()
}

#[test]
fn a_changed_mode_holds_from_the_next_turn_and_a_running_turn_keeps_its_own() {
    let place = place("settings-approval");
    std::fs::create_dir_all(&place.home).unwrap();
    let unjudged = "judge_model = \"off\"\n";
    writing(
        &place.home,
        "m",
        &format!("{unjudged}permissions = \"ask\"\n"),
    );
    let model = FakeServer::start(vec![
        bash("call_1", "git push --dry-run origin main"),
        Reply::tool_call(
            "call_2",
            "write",
            &serde_json::json!({"path": "first.txt", "content": "x"}),
        ),
        Reply::text("Done."),
        Reply::tool_call(
            "call_3",
            "write",
            &serde_json::json!({"path": "second.txt", "content": "x"}),
        ),
        Reply::text("Done."),
    ])
    .unwrap();
    let mut served = Served::start(&place, &model, &[]);
    let thread = new_thread(&mut served);
    let home = place.home.clone();
    let asked = turn_in(&mut served, &thread, |_| {
        // While the turn waits on the push, the person chooses full access.
        writing(
            &home,
            "m",
            &format!("{unjudged}permissions = \"full-access\"\n"),
        );
        "allow_once"
    });
    assert_eq!(
        asked, 2,
        "the push and the change asked: the running turn kept Ask first"
    );
    assert!(!result(&model, 2).contains("Not run"), "it ran");

    let asked = turn_in(&mut served, &thread, |_| "allow_once");
    assert_eq!(
        asked, 0,
        "the next turn runs in full access, which never asks"
    );
    served.end();
}

#[test]
fn turning_the_network_off_applies_to_the_next_turn_s_commands() {
    if !glave_computer::sandbox_available() {
        return;
    }
    let place = place("settings-network");
    std::fs::create_dir_all(&place.home).unwrap();
    writing(&place.home, "m", "judge_model = \"off\"\n");
    // A local listener only: no traffic leaves the machine.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connect = format!("exec 3<>/dev/tcp/127.0.0.1/{port} && echo connected");
    let model = FakeServer::start(vec![
        bash("call_1", &connect),
        Reply::text("Done."),
        bash("call_2", &connect),
        Reply::text("Done."),
    ])
    .unwrap();
    let mut served = Served::start(&place, &model, &[]);
    let thread = new_thread(&mut served);
    turn_in(&mut served, &thread, |_| "allow_once");
    assert!(result(&model, 1).contains("connected"), "the network is on");

    writing(&place.home, "m", "judge_model = \"off\"\nnetwork = false\n");
    turn_in(&mut served, &thread, |_| "allow_once");
    let second = result(&model, 3);
    assert!(!second.contains("connected"), "and now off: {second}");
    served.end();
}

#[test]
fn the_judge_follows_the_models_it_names_when_they_change() {
    // The judge decides in Auto behind the sandbox's wall.
    let required = std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_some();
    if !glave_computer::sandbox_available() && !required {
        return;
    }
    let place = place("settings-judge");
    std::fs::create_dir_all(&place.home).unwrap();
    writing(&place.home, "m", "judge_model = \"j\"\n");
    let push = "git push --dry-run origin main";
    let run = r#"{"decision":"run","reason":"the person asked to push"}"#;
    let model = FakeServer::start(vec![
        bash("call_1", push),
        Reply::text(run),
        Reply::text("Pushed."),
        bash("call_2", push),
        Reply::text(run),
        Reply::text("Pushed."),
    ])
    .unwrap();
    let mut served = Served::start(&place, &model, &[]);
    let thread = new_thread(&mut served);
    let asked = turn_in(&mut served, &thread, |_| "deny");
    assert_eq!(asked, 0, "the judge let it run");

    writing(&place.home, "n", "judge_model = \"k\"\n");
    let asked = turn_in(&mut served, &thread, |_| "deny");
    assert_eq!(asked, 0, "the new judge let it run too");
    let models: Vec<String> = model
        .received()
        .iter()
        .map(|request| request.body["model"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(models, ["m", "j", "m", "n", "k", "n"]);
    served.end();
}
