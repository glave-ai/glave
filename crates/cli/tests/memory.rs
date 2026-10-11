//! Memory end to end through the binary: off by default (the model is offered no memory tool),
//! on by the person's choice, a save shown at once, the next thread opening with it, and
//! `glave memory` listing and forgetting it.

#[cfg(test)]
#[path = "support/binary.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod memory {
    use std::process::Command;

    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::json;

    use super::support::*;

    fn memory_cli(place: &Place, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_glave"))
            .arg("memory")
            .args(args)
            .current_dir(&place.project)
            .env("GLAVE_HOME", &place.home)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        shown
    }

    #[test]
    fn off_by_default_the_model_is_offered_no_memory_tool() {
        let place = place("memory-off");
        let model = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
        let output = glave(&place, &model, &["--model", "m", "hello"], "");
        assert!(output.status.success(), "{}", text(&output));
        let tools = model.received()[0].body["tools"].to_string();
        assert!(!tools.contains("mcp__memory__"), "{tools}");
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
        let first = glave(&place, &model, &["--model", "m", "remember: tabs"], "");
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
}
