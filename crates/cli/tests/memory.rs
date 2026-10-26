//! Memory end to end through the binary: off when the person chooses it (the model is offered no
//! memory tool, and `glave memory` says it is off), on by the person's choice, a save shown at
//! once, the next thread opening with it, and `glave memory` listing and forgetting it, through
//! a server in its own process where none runs and through the person's server, in the project
//! of the folder it runs in.

use std::process::{Command, Output};

use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::http::{only_the_server_answers, serve_http};
use crate::support::*;

/// `glave memory` with `args` in `place`'s project folder, with no one at the terminal.
fn memory_run(place: &Place, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glave"))
        .arg("memory")
        .args(args)
        .current_dir(&place.project)
        .env("GLAVE_HOME", &place.home)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn memory_cli(place: &Place, args: &[&str]) -> String {
    let output = memory_run(place, args);
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    shown
}

#[test]
fn off_by_the_persons_choice_the_model_is_offered_no_memory_tool() {
    let place = place("memory-off");
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "memory = \"off\"\n").unwrap();
    let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
    let output = glave(&place, &model, &["--model", "m", "hello"], "");
    assert!(output.status.success(), "{}", text(&output));
    let tools = model.received()[0].body["tools"].to_string();
    assert!(!tools.contains("mcp__memory__"), "{tools}");
    // `glave memory` says so, as the app does.
    let listed = memory_run(&place, &[]);
    let shown = text(&listed);
    assert!(!listed.status.success(), "{shown}");
    assert!(shown.contains("memory is off"), "{shown}");
}

#[test]
fn a_save_shows_at_once_opens_the_next_thread_and_can_be_forgotten() {
    let place = place("memory-on");
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "memory = \"on\"\n").unwrap();
    let save = json!({"text": "Ada prefers tabs in Python", "scope": "personal"});
    let model = FakeServer::start(vec![
        Reply::tool_call("call_a", "mcp__memory__remember", &save),
        Reply::text("Noted."),
        Reply::text("Hello again."),
    ])
    .unwrap();
    // Unconfined and without asking, so the save goes through wherever the sandbox cannot.
    let args = ["--permissions", "full-access", "--model", "m"];
    let first = glave(
        &place,
        &model,
        &[&args[..], &["remember: tabs"]].concat(),
        "",
    );
    let shown = text(&first);
    assert!(first.status.success(), "{shown}");
    assert!(shown.contains("remembered:"), "{shown}");
    assert!(shown.contains("Ada prefers tabs in Python"), "{shown}");
    let second = glave(&place, &model, &["--model", "m", "hi"], "");
    assert!(second.status.success(), "{}", text(&second));
    let opening = said(&model.received()[2].body, "user").join("\n");
    assert!(opening.contains("# Memory"), "{opening}");
    assert!(opening.contains("Ada prefers tabs in Python"), "{opening}");

    let listed = memory_cli(&place, &[]);
    assert!(listed.contains("Ada prefers tabs in Python"), "{listed}");
    let id = listed.split_whitespace().next().unwrap().to_owned();
    let forgot = memory_cli(&place, &["forget", &id]);
    assert!(forgot.contains("forgot"), "{forgot}");
    let after = memory_cli(&place, &[]);
    assert!(after.contains("no memories here"), "{after}");
}

#[test]
fn through_the_person_s_server_memory_is_the_folder_s_project_s() {
    let place = place("memory-server");
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "memory = \"on\"\n").unwrap();
    // A second project, in a folder of its own, beside the one the server starts in.
    let other = Place {
        project: place.project.join("other"),
        home: place.home.clone(),
        lifeline: place.lifeline.clone(),
        folder: None,
    };
    std::fs::create_dir_all(&other.project).unwrap();
    let save = json!({"text": "this project ships on Fridays", "scope": "project"});
    let model = FakeServer::start(vec![
        Reply::tool_call("call_a", "mcp__memory__remember", &save),
        Reply::text("Noted."),
    ])
    .unwrap();
    let args = ["--permissions", "full-access", "--model", "m"];
    let saved = glave(
        &other,
        &model,
        &[&args[..], &["remember: fridays"]].concat(),
        "",
    );
    assert!(saved.status.success(), "{}", text(&saved));
    let (_served, _) = serve_http(&place, &model, &["--model", "m"]);
    only_the_server_answers(&place);

    let listed = memory_cli(&other, &[]);
    assert!(listed.contains("this project ships on Fridays"), "{listed}");
    // The folder the server started in is no project, and listing makes it none.
    let elsewhere = memory_cli(&place, &[]);
    assert!(
        elsewhere.contains(crate::folders::NO_PROJECT),
        "{elsewhere}"
    );
    let id = listed.split_whitespace().next().unwrap().to_owned();
    let forgot = memory_cli(&other, &["forget", &id]);
    assert!(forgot.contains("forgot"), "{forgot}");
    let after = memory_cli(&other, &[]);
    assert!(after.contains("no memories here"), "{after}");
}
