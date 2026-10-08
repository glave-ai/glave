//! The real `glave` binary, end to end, against a fake model server on 127.0.0.1: what a person
//! runs, what they see, what reaches the model, and what lands on disk.

#[cfg(test)]
mod glave_binary {
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    use glave_provider_openai::fake::{FakeServer, Reply};
    use serde_json::{Value, json};

    /// A project folder and a glave home for one test, inside the system temp folder.
    struct Place {
        project: PathBuf,
        home: PathBuf,
    }

    fn place(name: &str) -> Place {
        let dir = std::env::temp_dir().join(format!("glave-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("README.md"), "# demo\n").unwrap();
        Place {
            project,
            home: dir.join("home"),
        }
    }

    /// `glave` with `args`, talking only to `server`, with nothing from the person's own setup.
    fn glave(place: &Place, server: &FakeServer, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_glave"))
            .args(args)
            .current_dir(&place.project)
            .env("GLAVE_BASE_URL", server.url())
            .env("GLAVE_HOME", &place.home)
            .env("GLAVE_API_KEY", "sk-e2e-test")
            .env("NO_COLOR", "1")
            .env_remove("GLAVE_MODEL")
            .env_remove("GLAVE_PROVIDER")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn text(output: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    }

    /// The text of every message of `role` in a request the model received.
    fn said(body: &Value, role: &str) -> Vec<String> {
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == role)
            .map(|message| message["content"].to_string())
            .collect()
    }

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
            shown.contains("done · 1 model call · 0 tool calls"),
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
    fn with_yes_a_tool_call_changes_the_project() {
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
            &[
                "--model",
                "m",
                "--yes",
                "--events",
                &events_arg,
                "write notes",
            ],
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
    fn without_yes_and_no_one_to_ask_nothing_changes() {
        let place = place("declined");
        let server = FakeServer::start(vec![
            Reply::tool_call(
                "toolu_1",
                "write",
                &json!({"path": "notes.md", "content": "hello\n"}),
            ),
            Reply::text("I could not write it."),
        ])
        .unwrap();
        let output = glave(&place, &server, &["--model", "m", "write notes"], "");
        assert!(output.status.success(), "{}", text(&output));
        assert!(!place.project.join("notes.md").exists());
        let second = &server.received()[1].body;
        assert!(
            said(second, "tool")
                .iter()
                .any(|result| result.contains("declined")),
            "{second}"
        );
    }

    #[test]
    fn continue_picks_up_the_last_thread_and_a_new_run_starts_fresh() {
        let place = place("continue");
        let server = FakeServer::start(vec![
            Reply::text("Noted: marigold."),
            Reply::text("It was marigold."),
            Reply::text("Hello."),
        ])
        .unwrap();
        let first = glave(&place, &server, &["--model", "m", "remember marigold"], "");
        assert!(first.status.success(), "{}", text(&first));
        let second = glave(&place, &server, &["--model", "m", "-c", "what word?"], "");
        assert!(second.status.success(), "{}", text(&second));
        let third = glave(&place, &server, &["--model", "m", "hi"], "");
        assert!(third.status.success(), "{}", text(&third));
        let received = server.received();
        let resumed = &received[1].body;
        assert!(said(resumed, "user")[0].contains("remember marigold"));
        assert!(said(resumed, "assistant")[0].contains("Noted: marigold."));
        assert!(said(resumed, "user")[1].contains("what word?"));
        let fresh = &received[2].body;
        assert_eq!(said(fresh, "user").len(), 1, "a new thread: {fresh}");
    }

    #[test]
    fn an_interactive_session_reads_lines_until_exit() {
        let place = place("interactive");
        let server = FakeServer::start(vec![Reply::text("Hi there.")]).unwrap();
        let output = glave(
            &place,
            &server,
            &["--model", "fake-model"],
            "hello\n/exit\n",
        );
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(
            shown.contains("glave · fake-model via local ·"),
            "the banner names the model and where it is served: {shown}"
        );
        assert!(shown.contains("Hi there."), "{shown}");
        assert_eq!(server.received().len(), 1);
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
