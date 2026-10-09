//! The real `glave` binary, end to end, against a fake model server on 127.0.0.1: one-shot
//! runs, what a person sees, what reaches the model, and what lands on disk.

#[cfg(test)]
#[path = "support/binary.rs"]
mod support;

#[cfg(test)]
mod glave_binary {
    use std::path::Path;
    use std::process::Command;

    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::{Value, json};

    use super::support::*;

    #[test]
    fn a_one_shot_question_gets_an_answer_and_a_receipt() {
        let place = place("answer");
        let server = FakeServer::start(vec![Reply::text("The answer is 42.")]).unwrap();
        let output = glave(
            &place,
            &server,
            &["--model", "fake-model", "what is it?"],
            "",
        );
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("The answer is 42."), "{shown}");
        assert!(
            shown.contains("done · approval: auto · 1 model call · 0 tool calls"),
            "{shown}"
        );
        let received = server.received();
        assert_eq!(received.len(), 1);
        let body = &received[0].body;
        assert_eq!(body["model"], "fake-model");
        assert!(said(body, "user")[0].contains("what is it?"));
        assert!(
            said(body, "system")[0].contains("You are the AI model fake-model"),
            "the prompt names the model actually answering"
        );
        assert_eq!(
            received[0].authorization.as_deref(),
            Some("Bearer sk-e2e-test")
        );
    }

    #[test]
    fn a_tool_call_changes_the_project() {
        let place = place("tool");
        let server = FakeServer::start(vec![
            Reply::tool_call(
                "toolu_1",
                "write",
                &json!({"path": "notes.md", "content": "hello\n"}),
            ),
            Reply::text("I wrote notes.md."),
        ])
        .unwrap();
        let events = place.project.join("../events.jsonl");
        let events_arg = events.to_string_lossy().into_owned();
        let output = glave(
            &place,
            &server,
            &["--model", "m", "--events", &events_arg, "write notes"],
            "",
        );
        assert!(output.status.success(), "{}", text(&output));
        assert_eq!(
            std::fs::read_to_string(place.project.join("notes.md")).unwrap(),
            "hello\n"
        );
        let second = &server.received()[1].body;
        assert!(
            said(second, "tool")
                .iter()
                .any(|result| result.contains("notes.md")),
            "the model is told what the tool did: {second}"
        );
        let types: Vec<String> = std::fs::read_to_string(&events)
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            types,
            [
                "thread.started",
                "turn.started",
                "message.added",
                "tool.call.requested",
                "model.responded",
                "tool.call.completed",
                "message.added",
                "model.responded",
                "turn.ended",
            ]
        );
    }

    #[test]
    fn by_default_changes_and_commands_go_through() {
        let place = place("auto");
        let server = FakeServer::start(vec![
            Reply::tool_call(
                "toolu_1",
                "write",
                &json!({"path": "notes.md", "content": "hello\n"}),
            ),
            Reply::tool_call("toolu_2", "bash", &json!({"command": "touch ran.txt"})),
            Reply::text("Wrote notes.md and ran it."),
        ])
        .unwrap();
        let output = glave(
            &place,
            &server,
            &["--model", "m", "write notes, then run it"],
            "",
        );
        assert!(output.status.success(), "{}", text(&output));
        assert!(
            place.project.join("notes.md").exists(),
            "changes go through"
        );
        assert!(
            place.project.join("ran.txt").exists(),
            "commands go through"
        );
        assert!(
            text(&output).contains("approval: auto"),
            "{}",
            text(&output)
        );
    }

    #[test]
    fn in_ask_mode_with_no_one_to_ask_nothing_changes() {
        let place = place("ask");
        let server = FakeServer::start(vec![
            Reply::tool_call(
                "toolu_1",
                "write",
                &json!({"path": "notes.md", "content": "hello\n"}),
            ),
            Reply::text("I could not write it."),
        ])
        .unwrap();
        let output = glave(
            &place,
            &server,
            &["--model", "m", "--approval", "ask", "write notes"],
            "",
        );
        assert!(output.status.success(), "{}", text(&output));
        assert!(!place.project.join("notes.md").exists());
        let second = &server.received()[1].body;
        assert!(
            said(second, "tool")
                .iter()
                .any(|result| result.contains("No one is available to approve")),
            "{second}"
        );
        assert!(text(&output).contains("approval: ask"));
    }

    #[test]
    fn a_refused_key_fails_the_run_and_says_why() {
        let place = place("refused");
        let server = FakeServer::start(vec![Reply::Status(
            401,
            r#"{"error":{"message":"Missing Authentication header"}}"#.into(),
        )])
        .unwrap();
        let output = glave(&place, &server, &["--model", "m", "hi"], "");
        assert!(!output.status.success());
        let shown = text(&output);
        assert!(shown.contains("401"), "{shown}");
        assert!(shown.contains("Missing Authentication header"), "{shown}");
        assert!(!shown.contains(r#"{"error""#), "never raw JSON: {shown}");
        assert!(
            shown.contains("fix: 127.0.0.1 did not accept the key: check GLAVE_API_KEY"),
            "{shown}"
        );
    }

    #[test]
    fn an_unknown_model_names_the_id_to_check() {
        let place = place("unknown-model");
        let server = FakeServer::start(vec![Reply::Status(
            400,
            r#"{"error":{"message":"nosuch/model is not a valid model ID","code":400}}"#.into(),
        )])
        .unwrap();
        let output = glave(&place, &server, &["--model", "nosuch/model", "hi"], "");
        assert!(!output.status.success());
        let shown = text(&output);
        assert!(
            shown.contains("nosuch/model is not a valid model ID"),
            "{shown}"
        );
        assert!(
            shown.contains("fix: check the model id nosuch/model (--model or GLAVE_MODEL)"),
            "{shown}"
        );
        assert_eq!(
            server.received().len(),
            1,
            "not retried: it would fail again"
        );
    }

    #[test]
    fn a_failure_that_may_pass_is_retried_and_shown() {
        let place = place("retry");
        let server = FakeServer::start(vec![
            Reply::Status(503, r#"{"error":{"message":"overloaded"}}"#.into()),
            Reply::text("Answered after all."),
        ])
        .unwrap();
        let output = glave(&place, &server, &["--model", "m", "hi"], "");
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("retrying"), "{shown}");
        assert!(shown.contains("(1 of 4)"), "{shown}");
        assert!(shown.contains("Answered after all."), "{shown}");
        assert_eq!(server.received().len(), 2);
    }

    #[test]
    fn a_bad_idle_timeout_is_refused_before_anything_runs() {
        let place = place("idle");
        let server = FakeServer::start(Vec::new()).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_glave"))
            .args(["--model", "m", "hi"])
            .current_dir(&place.project)
            .env("GLAVE_BASE_URL", server.url())
            .env("GLAVE_HOME", &place.home)
            .env("GLAVE_IDLE_TIMEOUT", "soon")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            text(&output).contains("GLAVE_IDLE_TIMEOUT must be a number of seconds"),
            "{}",
            text(&output)
        );
        assert!(server.received().is_empty());
    }

    #[test]
    fn without_a_model_glave_explains_how_to_choose_one() {
        let place = place("no-model");
        let server = FakeServer::start(Vec::new()).unwrap();
        let output = glave(&place, &server, &["hi"], "");
        assert!(!output.status.success());
        assert!(
            text(&output).contains("no model chosen"),
            "{}",
            text(&output)
        );
        assert!(server.received().is_empty());
    }

    #[test]
    fn help_version_and_bad_flags() {
        let place = place("flags");
        let server = FakeServer::start(Vec::new()).unwrap();
        let help = glave(&place, &server, &["--help"], "");
        assert!(help.status.success());
        assert!(text(&help).contains("Usage:"));
        let version = glave(&place, &server, &["--version"], "");
        assert!(text(&version).starts_with("glave "));
        let bad = glave(&place, &server, &["--bogus"], "");
        assert_eq!(bad.status.code(), Some(2));
        assert!(text(&bad).contains("unknown option --bogus"));
        assert!(server.received().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_data_folder_is_private_and_holds_the_database() {
        use std::os::unix::fs::PermissionsExt as _;
        let place = place("home");
        let server = FakeServer::start(vec![Reply::text("ok")]).unwrap();
        std::fs::create_dir_all(&place.home).unwrap();
        std::fs::set_permissions(&place.home, std::fs::Permissions::from_mode(0o755)).unwrap();
        let output = glave(&place, &server, &["--model", "m", "hi"], "");
        assert!(output.status.success(), "{}", text(&output));
        let mode = std::fs::metadata(&place.home).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "tightened even though it existed");
        assert!(Path::new(&place.home).join("glave.db").exists());
    }
}
