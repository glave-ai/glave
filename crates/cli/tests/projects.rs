//! Every project alike, through the real binary over stdio with a fresh home
//! (`specs/one-server.md`, "Each project's own tools, hooks and settings"): two projects in one
//! server, each with a trusted server, hook, model and `[web]`, give each thread only its own;
//! the folder the server started in lends them nothing; and the person's own server reaches
//! threads in both projects and in their Chats.

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

/// `.mcp.json` bringing the test server as `name`.
fn bringing(name: &str) -> String {
    json!({ "mcpServers": { name: { "command": test_server() } } }).to_string()
}

/// A project folder `name` beside the one glave starts in, bringing the test server as
/// `name`, a hook and a model of its own, and `more` of its settings.
fn project(place: &Place, name: &str, more: &str) -> PathBuf {
    let folder = place.project.with_file_name(name);
    std::fs::create_dir_all(folder.join(".glave")).unwrap();
    std::fs::write(folder.join(".mcp.json"), bringing(name)).unwrap();
    let config = format!(
        "model = \"{name}-model\"\n{more}\n[[hooks.UserPromptSubmit]]\ncommand = \"true\"\n"
    );
    std::fs::write(folder.join(".glave/config.toml"), config).unwrap();
    std::fs::canonicalize(folder).unwrap()
}

/// The project whose folder is `folder`, made as the app's New project makes it.
fn project_of(served: &mut Served, folder: &Path) -> Value {
    let name = folder.file_name().unwrap().to_string_lossy();
    let path = folder.to_string_lossy();
    let params = json!({"name": name, "folder": {"kind": "path", "path": path}});
    served.call("project/create", &params)["result"]["project"]["id"].clone()
}

/// A thread in `project`, or in the person's Chats with none, its project trusted as the
/// person trusts it in the app, and one turn in it.
fn turn_in(served: &mut Served, project: Option<&Value>) -> Value {
    let params = project.map_or_else(|| json!({}), |project| json!({ "project": project }));
    let started = served.call("thread/start", &params);
    let thread = started["result"]["thread"].clone();
    if let Some(ask) = thread["trust"]["ask"].as_str() {
        let answer = json!({"thread": thread["id"], "ask": ask, "answer": "trust"});
        let answered = served.call("trust/answer", &answer);
        assert!(answered["result"]["thread"].is_object(), "{answered}");
    }
    let turn = json!({"thread": thread["id"], "prompt": "go"});
    let ran = served.call("turn/start", &turn);
    assert_eq!(ran["result"]["turn"]["outcome"], "completed", "{ran}");
    thread["id"].clone()
}

/// The model and the tools of a request the model received.
fn asked(body: &Value) -> (String, Vec<String>) {
    let tools = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect();
    (body["model"].as_str().unwrap().to_owned(), tools)
}

/// How many hooks ran in `thread`.
fn hooks_ran(served: &mut Served, thread: &Value) -> usize {
    let events = served.call("thread/events", &json!({ "thread": thread }));
    events["result"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "hook.ran")
        .count()
}

#[test]
fn each_project_s_server_hook_model_and_web_reach_its_own_threads_alone() {
    let place = place("projects-alike");
    std::fs::create_dir_all(&place.home).unwrap();
    // The folder glave starts in brings a server it trusted, which lends nothing to others.
    std::fs::write(place.project.join(".mcp.json"), bringing("start")).unwrap();
    trust_project(&place, &[]);
    // The person's own server, and the models they offer.
    std::fs::write(place.home.join("mcp.json"), bringing("mine")).unwrap();
    let mine = "model = \"m\"\nmemory = \"off\"\ntitles = false\n\
                    [[models]]\nid = \"alpha-model\"\n[[models]]\nid = \"beta-model\"\n";
    std::fs::write(place.home.join("config.toml"), mine).unwrap();
    let alpha = project(&place, "alpha", "[web]\nfetch = false\n");
    let beta = project(&place, "beta", "");
    let answers = (0..3).map(|_| Reply::text("Done.")).collect();
    let model = FakeServer::start(answers).unwrap();
    let mut served = Served::start(&place, &model, &["--sandbox", "off", "--approval", "auto"]);
    let (alpha, beta) = (
        project_of(&mut served, &alpha),
        project_of(&mut served, &beta),
    );
    let in_alpha = turn_in(&mut served, Some(&alpha));
    turn_in(&mut served, Some(&beta));
    let chat = turn_in(&mut served, None);
    let received: Vec<(String, Vec<String>)> = model
        .received()
        .iter()
        .map(|request| asked(&request.body))
        .collect();
    assert_eq!(received.len(), 3, "{received:?}");
    let has = |at: usize, tool: &str| received[at].1.iter().any(|name| name == tool);
    let servers = |at: usize| -> Vec<&str> {
        ["alpha", "beta", "start", "mine"]
            .into_iter()
            .filter(|name| has(at, &format!("mcp__{name}__echo")))
            .collect()
    };
    assert_eq!(received[0].0, "alpha-model");
    assert_eq!(servers(0), ["alpha", "mine"]);
    assert!(!has(0, "web_fetch"), "alpha's [web] turned it off");
    assert_eq!(received[1].0, "beta-model");
    assert_eq!(servers(1), ["beta", "mine"]);
    assert!(has(1, "web_fetch"), "the person's own [web]");
    assert_eq!(received[2].0, "m", "a chat: the default");
    assert_eq!(servers(2), ["mine"]);
    assert!(has(2, "web_fetch"));
    assert_eq!(hooks_ran(&mut served, &in_alpha), 1, "its own hook");
    assert_eq!(
        hooks_ran(&mut served, &chat),
        0,
        "no project's hook in a chat"
    );
    served.end();
}
