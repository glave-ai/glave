//! Someone invited to a person's own server (`specs/model.md`, "Invites"), the real binary at a
//! public origin: what the agent adds for them never reaches the owner's own folder, whose
//! `mcp.json` runs as the owner with the server's whole environment, and their commands never
//! leave the sandbox, which hides the server's token; the owner does both as ever.

use glave_event::{Body, ServiceAnswer, ThreadId, decode};
use glave_identity::testing::Authenticator;
use glave_ports::block_on;
use glave_provider_openai::fake::{FakeServer, Reply};
use glave_session::client::Remote;
use glave_session::protocol::{Notification, Request};
use glave_session::{Sent, TurnItem};
use serde_json::{Value, json};

use crate::connected::request;
use crate::http::serve_http_with;
use crate::support::*;

const SKILL: &str = "---\nname: notes\ndescription: Keeps notes.\n---\nKeep notes.\n";

/// What `method` with `params`, from a browser at `origin`, is answered with.
async fn from_browser(url: &str, origin: &str, method: &str, params: Value) -> Value {
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let answer = reqwest::Client::new()
        .post(format!("{url}/"))
        .header("host", origin.strip_prefix("http://").unwrap())
        .header("origin", origin)
        .header("glave-method", method)
        .json(&message)
        .send()
        .await
        .unwrap();
    let answer: Value = answer.json().await.unwrap();
    answer["result"].clone()
}

/// Someone who opened an invite `owner` made, with a passkey of their own, signed in.
async fn invited(owner: &Remote, url: &str, origin: &str) -> Remote {
    let made = request("invite/create", json!({"role": "member"}));
    let made = serde_json::to_value(owner.request(&made).await.unwrap()).unwrap();
    let params = json!({"code": made["code"], "link": "invite"});
    let asked = from_browser(url, origin, "passkey/create_options", params).await;
    let credential = Authenticator::new(origin)
        .unwrap()
        .create(&asked["options"])
        .unwrap();
    let params = json!({"ceremony": asked["ceremony"], "credential": credential, "name": "Phone"});
    let made = from_browser(url, origin, "passkey/create", params).await;
    Remote::new(url, made["session"].as_str().unwrap().to_owned()).unwrap()
}

/// Runs a turn in a new thread of `caller`, started with `params`, to its end, answering each
/// place glave asks about with the person's own folder: how many times it asked.
async fn turn(caller: &Remote, params: Value) -> usize {
    let started = request("thread/start", params);
    let started = serde_json::to_value(caller.request(&started).await.unwrap()).unwrap();
    let thread: ThreadId = serde_json::from_value(started["thread"]["id"].clone()).unwrap();
    let prompt = json!({"thread": thread, "prompt": "add these"});
    let Ok(Sent::Started(mut running)) = caller.start(&request("turn/start", prompt)).await else {
        panic!("a turn of their own");
    };
    let mut asks = 0;
    while let Some(item) = running.next().await {
        let TurnItem::Note(note) = item else { continue };
        let Notification::Event { event, .. } = note.as_ref() else {
            continue;
        };
        let decoded = decode(event.clone()).unwrap();
        let Body::ServiceAsked(asked) = decoded.body else {
            continue;
        };
        asks += 1;
        let values = json!({"place": "everywhere"}).as_object().unwrap().clone();
        let answer = Request::ServiceAnswer {
            thread,
            call: asked.call_id,
            asked: decoded.header.seq,
            answer: ServiceAnswer::Answered { values },
        };
        caller.request(&answer).await.unwrap();
    }
    asks
}

#[test]
fn someone_invited_adds_nothing_to_the_owner_s_own_folder_and_the_owner_still_can() {
    let place = place("invited-install");
    let reads_a_key = json!({"kind": "server", "name": "docs", "server": {
        "command": "/bin/sh", "args": ["-c", "true"], "env": {"LEAK": "${GLAVE_API_KEY}"}
    }});
    let skill = json!({"kind": "skill", "name": "notes", "skill": SKILL});
    let own = json!({"kind": "server", "name": "mine", "server": {"command": "true"}});
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "install", &reads_a_key),
        Reply::tool_call("call_2", "install", &skill),
        Reply::text("Nothing added."),
        Reply::tool_call("call_3", "install", &own),
        Reply::text("Added."),
    ])
    .unwrap();
    choose(&place, "serve", "{ origin = \"http://localhost\" }");
    let (_served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &[]);
    let (_, port) = url.rsplit_once(':').unwrap();
    let origin = format!("http://localhost:{port}");
    let token = std::fs::read_to_string(place.home.join("serve.token")).unwrap();
    let servers = place.home.join("mcp.json");
    block_on!(async {
        let owner = Remote::new(&url, token.trim().to_owned()).unwrap();
        let theirs = invited(&owner, &url, &origin).await;
        let asks = turn(&theirs, json!({})).await;
        assert!(!servers.exists(), "they wrote the owner's mcp.json");
        let skills = place.home.join("skills/notes");
        assert!(!skills.exists(), "they wrote the owner's skills");
        assert_eq!(asks, 0, "nowhere is theirs to choose");
        let told = model.received()[2].body.to_string();
        assert!(told.contains("Nothing can be added here"), "{told}");

        assert_eq!(turn(&owner, json!({})).await, 1);
        let written = std::fs::read_to_string(&servers).unwrap();
        assert!(written.contains("mine"), "{written}");
    });
}

#[test]
fn someone_invited_never_leaves_the_sandbox_on_the_server_s_computer_and_the_owner_still_can() {
    let place = place("invited-sandbox");
    let token_file = place.home.join("serve.token");
    let cat = format!("cat '{}'", token_file.display());
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "bash", &json!({"command": cat})),
        Reply::tool_call(
            "call_2",
            "bash",
            &json!({"command": cat, "outside_sandbox": true, "reason": "To read it."}),
        ),
        Reply::text("Could not."),
        Reply::tool_call("call_3", "bash", &json!({"command": cat})),
        Reply::text("Read it."),
    ])
    .unwrap();
    choose(&place, "serve", "{ origin = \"http://localhost\" }");
    let (_served, url, _) = serve_http_with(&place, &model, &["--model", "m"], &[]);
    let (_, port) = url.rsplit_once(':').unwrap();
    let origin = format!("http://localhost:{port}");
    let token = std::fs::read_to_string(&token_file).unwrap();
    let token = token.trim();
    let full = json!({"permissions": "full-access"});
    block_on!(async {
        let owner = Remote::new(&url, token.to_owned()).unwrap();
        let theirs = invited(&owner, &url, &origin).await;
        let found = theirs.request(&request("server/discover", json!({}))).await;
        let found = serde_json::to_value(found.unwrap()).unwrap();
        assert!(
            !found["modes"].to_string().contains("full-access"),
            "{found}"
        );
        let refused = theirs.request(&request("thread/start", full.clone())).await;
        let refused = serde_json::to_string(&refused.unwrap_err()).unwrap();
        assert!(
            refused.contains("only for this server's owner"),
            "{refused}"
        );

        assert_eq!(turn(&theirs, json!({})).await, 0, "nothing to ask them");
        let told: Vec<String> = model.received()[1..3]
            .iter()
            .map(|received| received.body.to_string())
            .collect();
        let read = told.iter().any(|body| body.contains(token));
        assert!(!read, "they read the owner's token");
        let refused = "only its owner's commands may run outside the sandbox";
        assert!(told[1].contains(refused), "{}", told[1]);

        assert_eq!(turn(&owner, full).await, 0);
        let read = model.received()[4].body.to_string().contains(token);
        assert!(read, "the owner reads their own token");
    });
}
