//! The eval runner against a fake model server: a task folder is copied, its turns sent through
//! the in-process harness, and the attempt graded by its checks and its test script.

use std::path::Path;
use std::sync::Arc;

use glave_eval::{Plan, Setup, attempt, load};
use glave_ports::block_on;
use glave_ports::testing::{FAKE_SECRET, TempDir};
use glave_provider_openai::Config;
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

const KEY: &str = FAKE_SECRET;

fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("eval-attempt-{name}")).unwrap()
}

/// A task that asks for a file to be written, graded by a test script and checks.
fn task(dir: &Path, glave: &str, test: &str) -> glave_eval::Task {
    let task = dir.join("task");
    std::fs::create_dir_all(task.join("environment")).unwrap();
    std::fs::create_dir_all(task.join("tests")).unwrap();
    std::fs::write(task.join("environment/README.md"), "# scratch\n").unwrap();
    std::fs::write(task.join("instruction.md"), "Write hello.txt saying hi.\n").unwrap();
    // Full access unless a test says otherwise, so the runner's own behaviour is the same on a
    // computer with no sandbox.
    let permissions = if glave.contains("permissions") {
        ""
    } else {
        "permissions = \"full-access\"\n"
    };
    std::fs::write(
        task.join("task.toml"),
        format!("[glave]\n{permissions}{glave}\n"),
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
        plans: false,
        subagents: false,
        memory: None,
        processes: glave_process::Processes::unrecorded(),
        office: true,
        glave_on_path: None,
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

/// glave's document skills are offered unless the run measures Python in their place.
#[test]
fn glave_s_document_skills_go_when_python_takes_their_place() {
    let offered = |office: bool| {
        let dir = folder(if office { "office" } else { "python" });
        let task = task(&dir, "", "grep -q hi hello.txt\n");
        let server = FakeServer::start(writes_hello()).unwrap();
        let mut setup = setup(&server);
        setup.office = office;
        let attempt = block_on!(attempt(&setup, &task, "fake-model", 1, &dir.join("run"))).unwrap();
        assert!(attempt.passed(), "{:?}", attempt.failures);
        let first = server.received().into_iter().next().unwrap();
        first.body.to_string().contains("glave:spreadsheets")
    };
    assert!(offered(true));
    assert!(!offered(false));
}

/// Each attempt starts with an empty Cargo home of its own, which goes with what it built.
#[test]
fn an_attempt_has_an_empty_cargo_home_of_its_own() {
    let dir = folder("cargo-home");
    let task = task(
        &dir,
        "",
        "test \"$CARGO_HOME\" = \"$(cd .. && pwd -P)/cargo-home\" && test -d \"$CARGO_HOME\" \
             && test -z \"$(ls -A \"$CARGO_HOME\")\"\n",
    );
    let server = FakeServer::start(writes_hello()).unwrap();
    let run = dir.join("run");
    let done = block_on!(attempt(&setup(&server), &task, "fake-model", 1, &run)).unwrap();
    assert_eq!(done.failures, Vec::<String>::new());
    assert!(!run.join("cargo-home").exists(), "it goes with the attempt");
}

/// What this machine's Cargo home downloaded is hidden from an attempt's commands, which could
/// otherwise build offline what a fresh computer would have to download (#823); the rest of
/// it, where `cargo` itself is, stays readable. Where CI expects the sandbox, it must run.
#[test]
fn this_machine_s_crate_downloads_are_hidden_from_an_attempt() {
    let sandbox = glave_computer::sandbox_available();
    let required = std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_some();
    assert!(
        sandbox || !required,
        "no sandbox runs here, which CI requires"
    );
    if !sandbox {
        return;
    }
    let dir = folder("downloads");
    let machine = dir.join("machine-cargo");
    for (folder, text) in [("registry", "canary-crate"), ("git", "canary-checkout")] {
        std::fs::create_dir_all(machine.join(folder)).unwrap();
        std::fs::write(machine.join(folder).join("lib.rs"), text).unwrap();
    }
    std::fs::create_dir_all(machine.join("bin")).unwrap();
    std::fs::write(machine.join("bin/cargo"), "cargo-readable").unwrap();
    let task = task(&dir, "permissions = \"auto\"", "exit 0\n");
    let reads = format!(
        "cat '{0}/registry/lib.rs' '{0}/git/lib.rs' '{0}/bin/cargo'; true",
        machine.display()
    );
    let server = FakeServer::start(vec![
        Reply::tool_call("call_1", "bash", &json!({ "command": reads })),
        Reply::text("Read them."),
    ])
    .unwrap();
    let mut setup = setup(&server);
    setup.environment.retain(|(name, _)| name != "CARGO_HOME");
    setup
        .environment
        .push(("CARGO_HOME".into(), machine.into()));
    let run = dir.join("run");
    let done = block_on!(attempt(&setup, &task, "fake-model", 1, &run)).unwrap();
    assert_eq!(done.failures, Vec::<String>::new());
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    assert!(events.contains("cargo-readable"), "{events}");
    assert!(!events.contains("canary-"), "{events}");
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

/// Each attempt has a temp folder of its own, which its setup and its grader share, and which
/// goes with it, read-only folders and all.
#[test]
fn an_attempt_has_a_temp_folder_of_its_own() {
    let dir = folder("temp");
    let task = task(
        &dir,
        r#"setup = ["echo set > \"$TMPDIR/from-setup\""]"#,
        "test \"$TMPDIR\" = \"$(cd .. && pwd -P)/tmp\" && test -f \"$TMPDIR/from-setup\" \
             && mkdir -p \"$TMPDIR/kept/a\" && chmod -R a-w \"$TMPDIR/kept\"\n",
    );
    let server = FakeServer::start(writes_hello()).unwrap();
    let run = dir.join("run");
    let done = block_on!(attempt(&setup(&server), &task, "fake-model", 1, &run)).unwrap();
    assert_eq!(done.failures, Vec::<String>::new());
    assert!(!run.join("tmp").exists(), "it goes with the attempt");
}

/// Two attempts of one task never run at once, though there is room for both: models write
/// scratch files at fixed paths in the shared `/tmp`, which they would overwrite (#750).
#[test]
fn attempts_of_one_task_take_turns() {
    let dir = folder("turns");
    let task = task(
        &dir,
        r#"setup = ["echo start >> \"$TURNS_LOG\""]"#,
        "echo end >> \"$TURNS_LOG\"\n",
    );
    let mut replies = writes_hello();
    replies.extend(writes_hello());
    let server = FakeServer::start(replies).unwrap();
    let mut setup = setup(&server);
    let log = dir.join("turns.log");
    setup
        .environment
        .push(("TURNS_LOG".into(), log.clone().into()));
    let plan = Plan {
        tasks: vec![task],
        models: vec!["a/one".into()],
        repeat: 2,
        concurrency: 2,
        max_cost_micros: 1_000_000,
        dir: dir.join("runs"),
    };
    let attempts = block_on!(glave_eval::run(Arc::new(setup), plan, |_| {}));
    assert!(
        attempts.iter().all(glave_eval::Attempt::passed),
        "{attempts:?}"
    );
    let turns = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        turns, "start\nend\nstart\nend\n",
        "one ends before the next starts"
    );
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

/// When each of the memory ledger's `memory.saved` events happened, in order.
fn saved_at(ledger: &str) -> Vec<jiff::Timestamp> {
    ledger
        .lines()
        .filter(|line| line.contains("\"type\":\"memory.saved\""))
        .map(|line| {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            event["at"].as_str().unwrap().parse().unwrap()
        })
        .collect()
}

#[test]
fn a_month_runs_on_its_days_in_one_store_and_grades_its_probes() {
    let dir = folder("month");
    // Tidy-up runs as the server runs it, in glave's own background tidier (tested in
    // glave-memory). Here nothing comes due, the only seeded memory being untrusted, so the fake
    // model's replies keep their order.
    let task = task(
        &dir,
        r#"memory = true
            tidy = "due"
            days_ago = 9
            probe_share = 0.5
            [[glave.memories]]
            text = "Ada writes British English"
            scope = "personal"
            untrusted = true
            days_ago = 10
            [[glave.threads]]
            project = "chats"
            days_ago = 1
            messages = ["Remember: I use Helix."]
            [[glave.threads]]
            messages = ["Which editor do I use?"]
            probe = [{ answer_contains = "Helix" }]
            [[glave.threads]]
            project = "beacon"
            messages = ["Which shell do I use?"]
            probe = [{ answer_contains = "fish" }]"#,
        "exit 0\n",
    );
    let remember = json!({"text": "Ada edits in Helix", "scope": "personal"});
    let server = FakeServer::start(vec![
        Reply::text("Hi."),
        Reply::tool_call("call_1", "mcp__memory__remember", &remember),
        Reply::text("Noted."),
        Reply::tool_call("call_2", "mcp__memory__recall", &json!({"query": "editor"})),
        Reply::text("Helix."),
        Reply::text("Bash, I think."),
    ])
    .unwrap();
    let run = dir.join("run");
    let attempt = block_on!(attempt(&setup(&server), &task, "fake-model", 1, &run)).unwrap();
    assert_eq!(
        attempt.failures,
        Vec::<String>::new(),
        "half the probes pass"
    );
    let probes = attempt.probes.unwrap();
    assert_eq!((probes.passed, probes.of), (1, 2));
    assert_eq!(
        probes.failed,
        ["probe 2 (thread 4): the answer does not mention \"fish\""]
    );
    assert_eq!(probes.recalled, [1]);
    let memory = attempt.memory.unwrap();
    assert_eq!(memory.tidy_runs, 0);
    assert_eq!(memory.tidied_away, Vec::<String>::new());
    assert_eq!(memory.used.keys().collect::<Vec<_>>(), ["personal"]);
    let shown = &memory.in_view;
    assert_eq!(shown.len(), 4, "one for each thread: {shown:?}");
    assert!(
        shown[0] > 0 && shown[0] == shown[1] && shown[2] > shown[1] && shown[3] == shown[2],
        "{shown:?}"
    );
    // Saved in Chats on day 1: nine days after the memory seeded ten days ago.
    let ledger = std::fs::read_to_string(run.join("memory.jsonl")).unwrap();
    let [seeded, helix] = saved_at(&ledger)[..] else {
        panic!("two memories saved: {ledger}");
    };
    let days = helix.duration_since(seeded).as_hours() / 24;
    assert_eq!(days, 9, "{ledger}");
    let transcript = std::fs::read_to_string(run.join("transcript.txt")).unwrap();
    assert!(
        transcript.contains("[thread 2: chats, days_ago 1]"),
        "{transcript}"
    );
}

/// memory-recall-paraphrase measures what it says it does: recall with the question's own words
/// finds nothing in this project, with the old thread's words it finds the fact, and an attempt
/// that does both passes the task's checks.
#[test]
fn the_paraphrased_fact_is_found_only_with_other_words() {
    let dir = folder("paraphrase");
    let task = load(Path::new("../../evals/tasks/memory-recall-paraphrase")).unwrap();
    let recall = |id: &str, query: &str| {
        Reply::tool_call(id, "mcp__memory__recall", &json!({ "query": query }))
    };
    let server = FakeServer::start(vec![
        recall("call_1", "ship staging box"),
        recall("call_2", "deploy preview server"),
        Reply::text("We deploy with Render, and the staging box is gannet-12."),
    ])
    .unwrap();
    let run = dir.join("run");
    let attempt = block_on!(attempt(&setup(&server), &task, "fake-model", 1, &run)).unwrap();
    assert_eq!(attempt.failures, Vec::<String>::new());
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    let found: Vec<&str> = (events.lines())
        .filter(|line| line.contains("\"type\":\"tool.call.completed\""))
        .collect();
    let [asked, reworded] = found[..] else {
        panic!("two recalls: {events}");
    };
    assert!(
        !asked.contains("gannet-12") && !asked.contains("heron-9"),
        "the question's words find nothing here: {asked}"
    );
    assert!(reworded.contains("gannet-12"), "{reworded}");
}
