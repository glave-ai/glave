//! Skills end to end: a repository set up for Claude Code brings its skills unchanged, `glave
//! skills` lists them with glave's own after, the model is offered `skill` and loads one, a
//! broken one is skipped with a warning naming its file, and `glave doc`, which glave's own
//! spreadsheets skill teaches, writes a workbook and reads it back.

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
    assert!(
        stdout.contains("\nglave:spreadsheets\n  Read, analyze and make spreadsheets")
            && stdout.contains("\n  built into glave"),
        "glave's own come after: {shown}"
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

#[test]
fn glave_doc_writes_a_workbook_and_reads_it_back_and_leaves_glaves_folder_alone() {
    let place = project("skills-doc");
    let model = FakeServer::start(vec![]).unwrap();
    let description = r#"{"sheets": [{"name": "Sales", "header": true,
        "rows": [["Region", "Revenue"], ["North", 1250.5], ["Total", "=SUM(B2:B2)"]]}]}"#;
    let written = glave(&place, &model, &["doc", "write", "sales.xlsx"], description);
    assert!(written.status.success(), "{}", text(&written));
    assert!(
        String::from_utf8_lossy(&written.stdout)
            .starts_with("Wrote sales.xlsx: Sales (3 rows, 1 formula).\n"),
        "{}",
        text(&written)
    );
    let read = run(&place, &model, &["doc", "read", "sales.xlsx"]);
    let shown = text(&read);
    assert!(read.status.success(), "{shown}");
    assert!(
        shown.contains("\n| 3 | Total | =SUM(B2:B2) → not computed yet |\n"),
        "{shown}"
    );
    // The CSV stays values only; what has none yet is said apart from it.
    let csv = run(&place, &model, &["doc", "csv", "sales.xlsx"]);
    assert!(csv.status.success(), "{}", text(&csv));
    assert_eq!(
        String::from_utf8_lossy(&csv.stdout),
        "Region,Revenue\nNorth,1250.5\nTotal,\n"
    );
    assert!(
        String::from_utf8_lossy(&csv.stderr)
            .starts_with("glave: formulas not computed yet, so their fields are empty: Sales!B3."),
        "{}",
        text(&csv)
    );
    let wrong = run(&place, &model, &["doc", "open", "sales.xlsx"]);
    assert_eq!(wrong.status.code(), Some(2), "{}", text(&wrong));
    assert!(
        text(&wrong).contains("glave doc write <file.xlsx>"),
        "{}",
        text(&wrong)
    );
    assert!(
        !place.home.exists(),
        "glave doc keeps out of glave's own folder"
    );
    assert!(model.received().is_empty(), "no model is asked");
}
