//! A project's `.mcp.json` and `.glave/config.toml`, end to end: servers a repository brings
//! stay off until the project is trusted, the model is told they are off, a recorded yes starts
//! them, and a repository with nothing that runs commands says nothing at all.

#[cfg(test)]
#[path = "support/binary.rs"]
mod support;

#[cfg(test)]
mod project {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::json;

    use super::support::*;

    /// The test server, built beside `glave` by `cargo xtask ci`.
    fn test_server() -> PathBuf {
        let path = Path::new(env!("CARGO_BIN_EXE_glave")).with_file_name("glave-mcp-test-server");
        assert!(
            path.exists(),
            "build the test server first: cargo build -p glave-mcp --features test-server"
        );
        path
    }

    /// The project brings the test server in its `.mcp.json`.
    fn bring_server(place: &Place) -> serde_json::Value {
        let server = json!({ "command": test_server() });
        let file = json!({ "mcpServers": { "tests": server } });
        std::fs::write(place.project.join(".mcp.json"), file.to_string()).unwrap();
        server
    }

    /// Records a yes for the project's `tests` server, as glave would after asking.
    fn trust(place: &Place, server: &serde_json::Value) {
        std::fs::create_dir_all(&place.home).unwrap();
        let root = std::fs::canonicalize(&place.project).unwrap();
        let record = format!(
            "version = 1\n\n[[project]]\npath = {:?}\n\n[project.items.\"mcp:tests\"]\n\
             definition = {:?}\ntrusted = true\n",
            root.display().to_string(),
            server.to_string()
        );
        std::fs::write(place.home.join("trust.toml"), record).unwrap();
    }

    fn offered(body: &serde_json::Value) -> Vec<String> {
        body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn an_untrusted_projects_servers_stay_off_and_the_model_is_told() {
        let place = place("project-untrusted");
        bring_server(&place);
        let model = FakeServer::start(vec![Reply::text("Fine.")]).unwrap();
        let output = glave(&place, &model, &["--model", "m", "hello"], "");
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(
            shown.contains("MCP server tests") && shown.contains("nobody is here to ask"),
            "{shown}"
        );
        let received = model.received();
        assert!(
            !offered(&received[0].body)
                .iter()
                .any(|t| t.contains("mcp__"))
        );
        let told = said(&received[0].body, "user").join("\n");
        assert!(
            told.contains("Project tools that are off") && told.contains("MCP server tests"),
            "{told}"
        );
        assert!(
            !place.home.join("trust.toml").exists(),
            "nothing is recorded without an answer"
        );
    }

    #[test]
    fn a_trusted_projects_servers_start_and_nothing_is_said() {
        let place = place("project-trusted");
        let server = bring_server(&place);
        trust(&place, &server);
        let model = FakeServer::start(vec![
            Reply::tool_call("call_1", "mcp__tests__echo", &json!({"text": "hi"})),
            Reply::text("It said hi."),
        ])
        .unwrap();
        let output = glave(&place, &model, &["--model", "m", "echo it"], "");
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("It said hi."), "{shown}");
        assert!(!shown.contains("is off"), "{shown}");
        let received = model.received();
        assert!(offered(&received[0].body).contains(&"mcp__tests__echo".to_owned()));
        let results = said(&received[1].body, "tool");
        assert!(results[0].contains("hi"), "{results:?}");
    }

    #[test]
    fn a_server_that_changed_since_it_was_trusted_is_off_again() {
        let place = place("project-changed");
        let server = bring_server(&place);
        trust(&place, &server);
        let other = json!({ "command": test_server(), "args": ["--different"] });
        let file = json!({ "mcpServers": { "tests": other } });
        std::fs::write(place.project.join(".mcp.json"), file.to_string()).unwrap();
        let model = FakeServer::start(vec![Reply::text("Fine.")]).unwrap();
        let output = glave(&place, &model, &["--model", "m", "hello"], "");
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("MCP server tests"), "{shown}");
        let received = model.received();
        assert!(
            !offered(&received[0].body)
                .iter()
                .any(|t| t.contains("mcp__"))
        );
    }

    #[test]
    fn a_repository_with_nothing_that_runs_commands_never_mentions_trust() {
        let place = place("project-plain");
        std::fs::create_dir_all(place.project.join(".glave")).unwrap();
        std::fs::write(
            place.project.join(".glave/config.toml"),
            "approval = \"ask\"\nmodel = \"from-project\"\n",
        )
        .unwrap();
        let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
        let output = glave(&place, &model, &["hello"], "");
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("approval: ask"), "applied: {shown}");
        assert!(
            !shown.contains("trust") && !shown.contains("is off"),
            "{shown}"
        );
        assert_eq!(model.received()[0].body["model"], "from-project");
        assert!(
            !said(&model.received()[0].body, "user")
                .join("")
                .contains("Project tools")
        );
    }

    #[test]
    fn policy_explain_shows_the_project_s_trust() {
        let place = place("project-explain");
        bring_server(&place);
        std::fs::create_dir_all(place.project.join(".glave")).unwrap();
        std::fs::write(
            place.project.join(".glave/config.toml"),
            "approval = \"auto\"\n",
        )
        .unwrap();
        let explain = || {
            Command::new(env!("CARGO_BIN_EXE_glave"))
                .args(["policy", "explain"])
                .current_dir(&place.project)
                .env("GLAVE_HOME", &place.home)
                .env("GLAVE_MODEL", "gemma4:12b")
                .env_remove("GLAVE_JUDGE_MODEL")
                .output()
                .unwrap()
        };
        let shown = text(&explain());
        assert!(shown.contains("Project "), "{shown}");
        assert!(shown.contains("MCP server tests"), "{shown}");
        assert!(shown.contains("glave will ask once"), "{shown}");
        assert!(shown.contains("settings (approval = \"auto\")"), "{shown}");
        assert!(
            !place.home.join("trust.toml").exists(),
            "explaining records nothing"
        );
        let server = json!({ "command": test_server() });
        trust(&place, &server);
        let shown = text(&explain());
        assert!(
            shown.contains("MCP server tests")
                && shown.contains("trusted (recorded in trust.toml)"),
            "{shown}"
        );
    }
}
