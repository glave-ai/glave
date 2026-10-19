//! Skills end to end: a repository set up for Claude Code brings its skills unchanged, `glave
//! skills` lists them, the model is offered `skill` and loads one, and a broken one is skipped
//! with a warning naming its file.

use std::process::{Command, Output, Stdio};

use glave_ports::testing::FAKE_SECRET;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::support::*;

/// A project with one good skill and one broken one, in Claude Code's folder.
fn project(name: &str) -> Place {
    let place = place(name);
    let skills = place.project.join(".claude/skills");
    std::fs::create_dir_all(skills.join("greet")).unwrap();
    std::fs::create_dir_all(skills.join("broken")).unwrap();
    std::fs::write(
        skills.join("greet/SKILL.md"),
        "---\nname: greet\ndescription: How to greet people in this project.\n---\n\
             Say ahoy.\n",
    )
    .unwrap();
    std::fs::write(skills.join("broken/SKILL.md"), "no frontmatter\n").unwrap();
    place
}

/// `glave` with `args`, with the person's home in the test's folder too.
fn run(place: &Place, server: &FakeServer, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(args)
        .current_dir(&place.project)
        .env("HOME", &place.home)
        .env("GLAVE_BASE_URL", server.url())
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_API_KEY", FAKE_SECRET)
        .env("NO_COLOR", "1")
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_PROVIDER")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn glave_skills_lists_them_and_warns_about_the_broken_one() {
    let place = project("skills-list");
    let model = FakeServer::start(vec![]).unwrap();
    let output = run(&place, &model, &["skills"]);
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("greet\n  How to greet people"),
        "{shown}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("broken/SKILL.md") && stderr.contains("frontmatter"),
        "{stderr}"
    );
    assert!(model.received().is_empty(), "listing asks no model");
}

#[test]
fn the_model_is_offered_skill_and_loads_one() {
    let place = project("skills-run");
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "skill", &json!({"name": "greet"})),
        Reply::text("Ahoy!"),
    ])
    .unwrap();
    let output = run(&place, &model, &["--model", "m", "greet me"]);
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    let received = model.received();
    let tools = received[0].body["tools"].as_array().unwrap();
    let skill = tools
        .iter()
        .find(|tool| tool["function"]["name"] == "skill")
        .unwrap();
    let description = skill["function"]["description"].as_str().unwrap();
    assert!(description.contains("<name>greet</name>"), "{description}");
    assert!(!description.contains("broken"), "{description}");
    let results = said(&received[1].body, "tool");
    assert!(results[0].contains("Say ahoy."), "{results:?}");
}
