//! A server that failed to start, started again at a later ask once its wait is over (#1199):
//! in the background, so nothing waits for it, and its tools back once it answers.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use glave_harness::Cancel;
use glave_mcp::{Limits, McpServer};
use glave_ports::settings::mcp::ServerConfig;
use glave_ports::testing::{ManualClock, TempDir};
use glave_ports::{Clock, InMemoryBlobs, ModelRequest, ToolSource, block_on};

use crate::history::{harness_for, scope, step};

const SERVER: &str = env!("CARGO_BIN_EXE_glave-mcp-test-server");

/// The longest an ask of a server starting again may take: it never waits for the start.
const PROMPTLY: Duration = Duration::from_secs(5);

/// A server run as `program`, which is not there yet, timed by `clock`.
fn launch(program: &std::path::Path, args: &[&str], clock: &Arc<ManualClock>) -> McpServer {
    let config = ServerConfig::Stdio {
        command: program.to_string_lossy().into_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        env: BTreeMap::new(),
    };
    let environment: BTreeMap<String, String> = std::env::vars().collect();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let processes = glave_process::Processes::unrecorded();
    McpServer::start(
        "later",
        &config,
        &environment,
        dir,
        Limits::default(),
        &processes,
    )
    .timed_by(Arc::clone(clock) as Arc<dyn Clock>)
}

#[test]
fn a_server_that_failed_to_start_is_started_again_once_its_wait_is_over_and_comes_back() {
    block_on!(async {
        let dir = TempDir::new("mcp-again").unwrap();
        let program = dir.path().join("server");
        let clock = Arc::new(ManualClock::at_second(1_000));
        let server = launch(&program, &[], &clock);
        let why = server.tools().await.unwrap_err();
        assert!(why.message.contains("could not run"), "{why}");
        // The program is there now, but the wait after a failure is not over.
        std::os::unix::fs::symlink(SERVER, &program).unwrap();
        clock.advance_seconds(9);
        server.tools().await.unwrap_err();
        assert!(!server.starting(), "started again before its wait was over");
        clock.advance_seconds(1);
        // This ask starts it again and goes on without it, saying why it was missing.
        let why = server.tools().await.unwrap_err();
        assert!(why.message.contains("could not run"), "{why}");
        server.started().await.unwrap();
        let tools = server.tools().await.unwrap();
        assert!(
            tools
                .iter()
                .any(|tool| tool.spec.name == "mcp__later__echo"),
            "{:?}",
            tools.iter().map(|tool| &tool.spec.name).collect::<Vec<_>>()
        );
        server.close().await;
    });
}

#[test]
fn nothing_waits_for_a_server_starting_again_and_closing_it_stops_it() {
    block_on!(async {
        let dir = TempDir::new("mcp-again").unwrap();
        let program = dir.path().join("server");
        let clock = Arc::new(ManualClock::at_second(1_000));
        // Once there, it never answers: the start again would take the whole start limit.
        let server = launch(&program, &["60"], &clock);
        server.tools().await.unwrap_err();
        std::os::unix::fs::symlink("/bin/sleep", &program).unwrap();
        clock.advance_seconds(10);
        server.tools().await.unwrap_err();
        assert!(server.starting());
        let asked = tokio::time::timeout(PROMPTLY, server.tools()).await;
        let why = asked.expect("an ask waited for a start again").unwrap_err();
        assert!(why.message.contains("starting again"), "{why}");
        server.close().await;
        assert!(!server.starting());
        let why = server.started().await.unwrap_err();
        assert!(why.to_string().contains("it was closed"), "{why}");
        // Closed, it never starts again.
        clock.advance_seconds(3_600);
        server.tools().await.unwrap_err();
        assert!(!server.starting());
    });
}

/// What a request opens with, as the provider's prompt cache reads it.
fn prefix(request: &ModelRequest) -> (String, Vec<u8>) {
    let tools = serde_json::to_vec(&request.tools).unwrap();
    (request.system.clone(), tools)
}

/// A server of the person's own on this computer that comes back mid-thread comes as a
/// connection turned on does (`specs/invariants.md`, 27): the prefix is made again once, with
/// its tools, and kept from then on.
#[test]
fn a_local_server_back_mid_thread_makes_the_prefix_again_once() {
    block_on!(async {
        let dir = TempDir::new("mcp-again-thread").unwrap();
        let program = dir.path().join("server");
        let clock = Arc::new(ManualClock::at_second(1_000));
        let server = Arc::new(launch(&program, &[], &clock));
        let source = Arc::clone(&server) as Arc<dyn ToolSource>;
        let answers = (0..4).map(|_| step(None)).collect();
        let (harness, seen) =
            harness_for(dir.path(), InMemoryBlobs::new(), answers, vec![source]).await;
        let mut thread = harness.start_thread(scope()).await.unwrap();
        let mut turn = async |said: &str| {
            let (cancel, mut heard) = (Cancel::new(), |_| {});
            let ran = harness.run_turn(&mut thread, said, &cancel, &mut heard);
            ran.await.unwrap();
        };
        turn("hello").await;
        std::os::unix::fs::symlink(SERVER, &program).unwrap();
        clock.advance_seconds(10);
        // This turn starts it again and goes on without it.
        turn("and now?").await;
        server.started().await.unwrap();
        turn("now?").await;
        turn("still?").await;
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 4);
        assert!(
            prefix(&requests[1]) == prefix(&requests[0]),
            "it was not back yet"
        );
        assert!(
            prefix(&requests[2]) != prefix(&requests[0]),
            "made again once it was back"
        );
        assert!(
            prefix(&requests[3]) == prefix(&requests[2]),
            "and only once"
        );
        let echo = |at: usize| {
            requests[at]
                .tools
                .iter()
                .any(|tool| tool.name == "mcp__later__echo")
        };
        assert!(!echo(1) && echo(2), "its tools came with it");
        drop(harness);
        server.close().await;
    });
}
