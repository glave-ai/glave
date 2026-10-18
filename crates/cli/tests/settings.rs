//! The person's settings read again, through the real binary over stdio with a fresh home
//! (`specs/one-server.md`, "Its settings"): an edited `config.toml` changes the next turn's
//! model, and a server added to `mcp.json` starts while a removed one stops, with no restart.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
#[path = "support/served.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod served;

#[cfg(test)]
mod settings {
    use std::path::{Path, PathBuf};

    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::{Value, json};

    use super::served::*;
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
        let mut served = Served::start(&place, &model, &["--sandbox", "off", "--approval", "auto"]);
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
}
