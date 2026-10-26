//! `config.toml` in glave's folder, end to end: its choices reach the run, flags and the
//! environment win over it, a typo stops glave before anything runs, and `glave policy explain`
//! says where each setting came from.

use std::process::Command;

use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::support::*;

fn choose(place: &Place, text: &str) {
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), text).unwrap();
}

#[test]
fn the_person_s_choices_reach_the_run_and_flags_win_over_them() {
    let place = place("config-choices");
    choose(&place, "model = \"from-file\"\npermissions = \"ask\"\n");
    let model = FakeServer::start(vec![Reply::text("Hi."), Reply::text("Hi again.")]).unwrap();
    let output = glave(&place, &model, &["hello"], "");
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(shown.contains("permissions: ask"), "{shown}");
    assert_eq!(model.received()[0].body["model"], "from-file");
    let output = glave(
        &place,
        &model,
        &["--permissions", "full-access", "--model", "m", "hi"],
        "",
    );
    let shown = text(&output);
    assert!(shown.contains("permissions: full-access"), "{shown}");
    assert_eq!(model.received()[1].body["model"], "m");
}

#[test]
fn an_unknown_key_stops_glave_naming_the_file_and_the_key() {
    let place = place("config-typo");
    choose(&place, "aproval = \"ask\"\n");
    let model = FakeServer::start(vec![Reply::text("never")]).unwrap();
    let output = glave(&place, &model, &["--model", "m", "hello"], "");
    let shown = text(&output);
    assert!(!output.status.success(), "{shown}");
    assert!(
        shown.contains("config.toml") && shown.contains("aproval"),
        "{shown}"
    );
    assert!(model.received().is_empty());
}

#[test]
fn only_the_mcp_servers_chosen_start() {
    let place = place("config-mcp");
    let servers = json!({ "mcpServers": { "broken": { "command": "/nonexistent/x" } } });
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("mcp.json"), servers.to_string()).unwrap();
    choose(&place, "mcp = []\n");
    let model = FakeServer::start(vec![Reply::text("Done.")]).unwrap();
    let output = glave(&place, &model, &["--model", "m", "hello"], "");
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(!shown.contains("broken"), "the server stays off: {shown}");
    choose(&place, "mcp = [\"brokn\"]\n");
    let output = glave(&place, &model, &["--model", "m", "hello"], "");
    let shown = text(&output);
    assert!(!output.status.success(), "{shown}");
    assert!(
        shown.contains("brokn") && shown.contains("mcp.json"),
        "{shown}"
    );
}

#[test]
fn policy_explain_names_where_each_setting_came_from() {
    let place = place("config-explain");
    choose(&place, "permissions = \"ask\"\n");
    let output = Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(["policy", "explain", "--no-network"])
        .current_dir(&place.project)
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_MODEL", "gemma4:12b")
        .env_remove("GLAVE_JUDGE_MODEL")
        .output()
        .unwrap();
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    let config = place.home.join("config.toml");
    let lines: Vec<&str> = shown.lines().collect();
    let row = |name: &str| {
        lines
            .iter()
            .find(|line| line.trim_start().starts_with(name))
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    };
    assert_eq!(row("model"), "model gemma4:12b from GLAVE_MODEL");
    let permissions = row("permissions");
    assert!(
        permissions.starts_with("permissions ask")
            && permissions.ends_with(&format!("from {}", config.display())),
        "{permissions}"
    );
    assert_eq!(row("network"), "network off from --network or --no-network");
}
