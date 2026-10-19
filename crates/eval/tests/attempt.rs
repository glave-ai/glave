//! The eval runner against a fake model server: a task folder is copied, its turns sent through
//! the in-process harness, and the attempt graded by its checks and its test script.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_eval::{Plan, Setup, attempt, load};
use glave_ports::block_on;
use glave_ports::testing::FAKE_SECRET;
use glave_provider_openai::Config;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

const KEY: &str = FAKE_SECRET;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("glave-eval-attempt-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A task that asks for a file to be written, graded by a test script and checks.
fn task(dir: &Path, glave: &str, test: &str) -> glave_eval::Task {
    let task = dir.join("task");
    std::fs::create_dir_all(task.join("environment")).unwrap();
    std::fs::create_dir_all(task.join("tests")).unwrap();
    std::fs::write(task.join("environment/README.md"), "# scratch\n").unwrap();
    std::fs::write(task.join("instruction.md"), "Write hello.txt saying hi.\n").unwrap();
    // Unconfined and without asking unless a test says otherwise, so the runner's own
    // behaviour is the same on a computer with no sandbox.
    let sandbox = if glave.contains("sandbox") {
        ""
    } else {
        "sandbox = \"off\"\n"
    };
    let approval = if glave.contains("approval") {
        ""
    } else {
        "approval = \"auto\"\n"
    };
    std::fs::write(
        task.join("task.toml"),
        format!("[glave]\n{sandbox}{approval}{glave}\n"),
    )
    .unwrap();
    std::fs::write(task.join("tests/test.sh"), test).unwrap();
    load(&task).unwrap()
}

fn setup(server: &FakeServer) -> Setup {
    let mut environment: Vec<_> = std::env::vars_os().collect();
    environment.push(("GLAVE_API_KEY".into(), KEY.into()));
    Setup {
        endpoint: Config::for_url(server.url(), Some(KEY.to_owned())).unwrap(),
        environment,
        todo: false,
        subagents: false,
        memory: None,
    }
}

fn writes_hello() -> Vec<Reply> {
    vec![
        Reply::tool_call(
            "call_1",
            "write",
            &json!({"path": "hello.txt", "content": "hi\n"}),
        ),
        Reply::text("I wrote hello.txt."),
    ]
}

#[test]
fn a_task_done_right_passes_and_leaves_its_record() {
    let dir = folder("passes");
    let task = task(
        &dir,
        r#"checks = [{ answer_contains = "hello.txt" }, { exists = "hello.txt" }, { kept = "README.md" }]"#,
        // The grader sees the workspace, never glave's endpoint variables, and builds in
        // the run's own folder, outside the workspace.
        "grep -q hi hello.txt && test -z \"${GLAVE_API_KEY:-}\"\n\
             mkdir -p \"$CARGO_TARGET_DIR\"\n\
             test \"$(cd \"$CARGO_TARGET_DIR\" && pwd -P)\" = \"$(cd .. && pwd -P)/target\"\n",
    );
    let server = FakeServer::start(writes_hello()).unwrap();
    let run = dir.join("run");
    let attempt = block_on!(attempt(&setup(&server), &task, "fake-model", 1, &run)).unwrap();
    assert_eq!(attempt.failures, Vec::<String>::new());
    assert!(attempt.passed());
    assert_eq!((attempt.model_calls, attempt.tool_calls), (2, 1));
    assert_eq!(attempt.input_tokens, 200);
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    assert!(events.contains("\"type\":\"turn.ended\""));
    let transcript = std::fs::read_to_string(run.join("transcript.txt")).unwrap();
    assert!(
        transcript.contains("> Write hello.txt saying hi."),
        "{transcript}"
    );
    assert!(transcript.contains("[tool]"), "{transcript}");
    assert!(run.join("test.log").is_file());
    assert!(
        !run.join("target").exists(),
        "what the run built is removed"
    );
    let sent = server.received();
    assert_eq!(
        sent.first().and_then(|r| r.authorization.as_deref()),
        Some(format!("Bearer {KEY}").as_str())
    );
}

#[test]
fn every_failure_is_named() {
    let dir = folder("fails");
    let task = task(
        &dir,
        r#"checks = [{ answer_contains = "goodbye" }, { missing = "hello.txt" }]"#,
        "echo 'expected goodbye.txt'\nexit 1\n",
    );
    let server = FakeServer::start(writes_hello()).unwrap();
    let attempt = block_on!(attempt(
        &setup(&server),
        &task,
        "fake-model",
        1,
        &dir.join("run"),
    ))
    .unwrap();
    assert!(!attempt.passed());
    let failures = attempt.failures.join("\n");
    assert!(
        failures.contains("tests/test.sh failed: expected goodbye.txt"),
        "{failures}"
    );
    assert!(
        failures.contains("the answer does not mention \"goodbye\""),
        "{failures}"
    );
    assert!(failures.contains("hello.txt was created"), "{failures}");
}

#[test]
fn later_turns_continue_the_thread_from_its_log() {
    let dir = folder("turns");
    let task = task(
        &dir,
        r#"turns = ["And now?"]
            checks = ["one_thread", { answer_contains = "still here" }]"#,
        "exit 0\n",
    );
    let mut replies = writes_hello();
    replies.push(Reply::text("Still here."));
    let server = FakeServer::start(replies).unwrap();
    let attempt = block_on!(attempt(
        &setup(&server),
        &task,
        "fake-model",
        1,
        &dir.join("run"),
    ))
    .unwrap();
    assert_eq!(attempt.failures, Vec::<String>::new());
    let last = server.received().pop().unwrap().body;
    let messages = last["messages"].as_array().unwrap();
    assert!(
        messages.len() >= 5,
        "the resumed thread sends the whole conversation: {messages:?}"
    );
}

#[test]
fn a_correction_sent_while_the_turn_runs_reaches_the_model_at_its_next_call() {
    let dir = folder("steer");
    let steered = task(
        &dir,
        r#"steer = { after = 1, message = "Make it say hello instead." }
            checks = [{ sent = "Make it say hello instead." }]"#,
        "exit 0\n",
    );
    let server = FakeServer::start(writes_hello()).unwrap();
    let run = dir.join("run");
    let done = block_on!(attempt(&setup(&server), &steered, "fake-model", 1, &run)).unwrap();
    assert_eq!(done.failures, Vec::<String>::new());
    assert_eq!(done.model_calls, 2, "read in the same turn");
    let second = server.received().pop().unwrap().body;
    let last = second["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"], "Make it say hello instead.");
    let transcript = std::fs::read_to_string(run.join("transcript.txt")).unwrap();
    assert!(
        transcript.contains("> (while it works) Make it say hello instead."),
        "{transcript}"
    );

    // A turn that calls no tool never gets it, and says so.
    let quiet = folder("steer-unsent");
    let unsent = task(
        &quiet,
        "steer = { after = 1, message = \"Too late.\" }",
        "exit 0\n",
    );
    let server = FakeServer::start(vec![Reply::text("Done.")]).unwrap();
    let run = quiet.join("run");
    let done = block_on!(attempt(&setup(&server), &unsent, "fake-model", 1, &run)).unwrap();
    assert_eq!(
        done.failures,
        [
            "the correction was never sent: the first turn ended with 0 tool calls finished, and \
              it waits for 1"
        ]
    );
}

#[test]
fn later_threads_share_one_store_with_memory_and_the_checks_read_it() {
    let dir = folder("threads");
    let task = task(
        &dir,
        r#"memory = true
            checks = [{ memory_holds = "tabs" }, { personal_memory_lacks = ["tabs"] }]
            [[glave.memories]]
            text = "Ada likes short answers"
            scope = "personal"
            [[glave.threads]]
            messages = ["Which indentation do we use?"]"#,
        "exit 0\n",
    );
    let mut replies = writes_hello();
    replies.insert(
        1,
        Reply::tool_call(
            "call_m",
            "mcp__memory__remember",
            &json!({"text": "This project uses tabs", "scope": "project"}),
        ),
    );
    replies.push(Reply::text("Tabs."));
    let server = FakeServer::start(replies).unwrap();
    let attempt = block_on!(attempt(
        &setup(&server),
        &task,
        "fake-model",
        1,
        &dir.join("run"),
    ))
    .unwrap();
    assert_eq!(attempt.failures, Vec::<String>::new());
    let last = server.received().pop().unwrap().body.to_string();
    assert!(last.contains("# Memory"), "{last}");
    assert!(last.contains("This project uses tabs"), "{last}");
    assert!(last.contains("Ada likes short answers"), "{last}");
    let ledger = std::fs::read_to_string(dir.join("run/memory.jsonl")).unwrap();
    assert!(ledger.contains("memory.saved"), "{ledger}");
}

#[test]
fn a_model_that_errors_fails_the_attempt() {
    let dir = folder("errors");
    let task = task(&dir, "", "exit 0\n");
    let server = FakeServer::start(vec![Reply::Status(500, "down".into())]).unwrap();
    let attempt = block_on!(attempt(
        &setup(&server),
        &task,
        "fake-model",
        1,
        &dir.join("run"),
    ))
    .unwrap();
    assert!(!attempt.passed());
    assert!(
        attempt.failures.iter().any(|f| f.starts_with("turn 1")),
        "{:?}",
        attempt.failures
    );
}

#[test]
fn a_failed_setup_command_is_an_error_not_a_result() {
    let dir = folder("setup");
    let task = task(&dir, r#"setup = ["false"]"#, "exit 0\n");
    let server = FakeServer::start(Vec::new()).unwrap();
    let result = block_on!(attempt(
        &setup(&server),
        &task,
        "fake-model",
        1,
        &dir.join("run"),
    ));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("setup `false` failed")
    );
    assert!(server.received().is_empty(), "the model is never called");
}

#[test]
fn nothing_starts_once_the_cost_cap_is_reached() {
    let dir = folder("cap");
    let task = task(&dir, "", "exit 0\n");
    let server = FakeServer::start(Vec::new()).unwrap();
    let plan = Plan {
        tasks: vec![task],
        models: vec!["a/one".into(), "b/two".into()],
        repeat: 2,
        concurrency: 2,
        max_cost_micros: 0,
        dir: dir.join("runs"),
    };
    let mut seen = 0;
    let attempts = block_on!(glave_eval::run(Arc::new(setup(&server)), plan, |_| {
        seen += 1;
    }));
    assert_eq!(seen, 4);
    assert!(attempts.iter().all(|a| a.skipped));
    let order: Vec<_> = attempts
        .iter()
        .map(|a| (a.model.as_str(), a.number))
        .collect();
    assert_eq!(
        order,
        [("a/one", 1), ("a/one", 2), ("b/two", 1), ("b/two", 2)]
    );
    assert!(server.received().is_empty());
}
