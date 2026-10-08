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
        let mut child = start(place, server, args);
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// `glave` started with `args`, still running.
    fn start(place: &Place, server: &FakeServer, args: &[&str]) -> std::process::Child {
        Command::new(env!("CARGO_BIN_EXE_glave"))
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
            .unwrap()
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
            shown.contains("done · approval: edits · 1 model call · 0 tool calls"),
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
    fn agents_md_from_the_project_and_the_person_reaches_the_model() {
        let place = place("agents-md");
        std::fs::write(place.project.join("AGENTS.md"), "Run make check.\n").unwrap();
        std::fs::create_dir_all(&place.home).unwrap();
        std::fs::write(place.home.join("AGENTS.md"), "Answer briefly.\n").unwrap();
        let server = FakeServer::start(vec![Reply::text("Hi.")]).unwrap();
        let output = glave(&place, &server, &["--model", "m"], "hello\n/exit\n");
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(
            shown.contains("following ") && shown.contains("AGENTS.md"),
            "the banner names the files: {shown}"
        );
        let first = said(&server.received()[0].body, "user").join("\n");
        let order = [
            "# AGENTS.md instructions",
            "Answer briefly.",
            "Run make check.",
            "hello",
        ]
        .map(|text| first.find(text).unwrap_or(usize::MAX));
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "the person's file, then the project's, then the prompt: {first}"
        );
        assert!(!said(&server.received()[0].body, "system")[0].contains("Run make check."));
    }

    #[cfg(unix)]
    #[test]
    fn ctrl_c_cancels_the_turn_and_the_thread_carries_on() {
        let place = place("ctrl-c");
        let server = FakeServer::start(vec![Reply::Stall, Reply::text("Still here.")]).unwrap();
        let events = place.project.join("../events.jsonl");
        let events_arg = events.to_string_lossy().into_owned();
        let child = start(
            &place,
            &server,
            &["--model", "m", "--events", &events_arg, "think hard"],
        );
        assert!(
            server.wait_for_requests(1, std::time::Duration::from_secs(30)),
            "the model was asked"
        );
        let sent = Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
        let output = child.wait_with_output().unwrap();
        let shown = text(&output);
        assert_eq!(output.status.code(), Some(130), "{shown}");
        assert!(shown.contains("cancelled"), "{shown}");
        let recorded = std::fs::read_to_string(&events).unwrap();
        assert!(recorded.contains("\"outcome\":\"cancelled\""), "{recorded}");

        let output = glave(
            &place,
            &server,
            &["--model", "m", "--continue", "are you there?"],
            "",
        );
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(shown.contains("Still here."), "{shown}");
        let asked = said(&server.received()[1].body, "user").join("\n");
        assert!(
            asked.contains("think hard") && asked.contains("cancelled this turn"),
            "the next turn knows the last was cancelled: {asked}"
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
    fn in_edits_mode_file_changes_land_but_commands_wait_for_a_person() {
        let place = place("edits");
        let server = FakeServer::start(vec![
            Reply::tool_call(
                "toolu_1",
                "write",
                &json!({"path": "notes.md", "content": "hello\n"}),
            ),
            Reply::tool_call("toolu_2", "bash", &json!({"command": "touch ran.txt"})),
            Reply::text("Wrote notes.md; the command needs a person."),
        ])
        .unwrap();
        let output = glave(
            &place,
            &server,
            &["--model", "m", "write notes, then run it"],
            "",
        );
        assert!(output.status.success(), "{}", text(&output));
        assert!(place.project.join("notes.md").exists(), "edits go through");
        assert!(!place.project.join("ran.txt").exists(), "commands wait");
        assert!(
            text(&output).contains("bash: touch ran.txt … not run: no one here to approve it"),
            "the log shows what did not run: {}",
            text(&output)
        );
        let third = &server.received()[2].body;
        assert!(
            said(third, "tool")
                .iter()
                .any(|result| result.contains("No one is available to approve")),
            "the model learns no one is there: {third}"
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
    fn slash_compact_summarizes_the_session() {
        let place = place("compact");
        let server = FakeServer::start(vec![
            Reply::text("Hi there."),
            Reply::text("We greeted each other."),
            Reply::text("You said hello."),
        ])
        .unwrap();
        let output = glave(
            &place,
            &server,
            &["--model", "m"],
            "hello\n/compact\nwhat did I say?\n/exit\n",
        );
        let shown = text(&output);
        assert!(output.status.success(), "{shown}");
        assert!(
            shown.contains("compacted the conversation to fit"),
            "{shown}"
        );
        let after = &server.received()[2].body;
        let user = said(after, "user");
        // One message, as strict chat templates need: the summary, then the new question.
        assert_eq!(user.len(), 1, "{after}");
        let (_, later) = user[0].split_once("We greeted each other.").unwrap();
        assert!(later.contains("what did I say?"), "{after}");
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
