//! glave never leaves an orphaned process (#361), end to end: a glave killed mid-turn, with a
//! command running, one kept in the background and an MCP server started, leaves them
//! recorded, and the next glave to start stops every one.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::{Value, json};

use crate::support::*;

/// The test server, built beside `glave` by `cargo xtask ci`.
fn test_server() -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_glave")).with_file_name("glave-mcp-test-server");
    assert!(
        path.exists(),
        "build the test server first: cargo build -p glave-mcp --features test-server"
    );
    path
}

/// The ledger's entries: each process's id and owner.
fn recorded(home: &Path) -> Vec<(u32, String)> {
    let Ok(listed) = std::fs::read_dir(home.join("run/processes")) else {
        return Vec::new();
    };
    listed
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| std::fs::read(path).ok())
        .filter_map(|text| serde_json::from_slice::<Value>(&text).ok())
        .map(|entry| {
            let pid = u32::try_from(entry["process"]["pid"].as_u64().unwrap()).unwrap();
            (pid, entry["owner"]["kind"].as_str().unwrap().to_owned())
        })
        .collect()
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// Waits up to `limit` for `done`.
fn until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while !done() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

#[test]
fn a_killed_glave_s_commands_and_servers_are_stopped_at_the_next_start() {
    let place = place("orphans");
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "memory = \"off\"\n").unwrap();
    let servers = json!({ "mcpServers": { "tests": { "command": test_server() } } });
    std::fs::write(place.home.join("mcp.json"), servers.to_string()).unwrap();
    let model = FakeServer::start(vec![
        Reply::tool_call(
            "call_1",
            "bash",
            &json!({"command": "sleep 300", "background": true}),
        ),
        Reply::tool_call("call_2", "bash", &json!({"command": "sleep 301"})),
        Reply::text("never reached"),
    ])
    .unwrap();
    // Unconfined, so nothing but the ledger can stop what it leaves.
    let args = [
        "--model",
        "m",
        "--sandbox",
        "off",
        "--approval",
        "auto",
        "go",
    ];
    let mut killed = start(&place, &model, &args);
    let running = until(Duration::from_secs(60), || recorded(&place.home).len() == 3);
    let entries = recorded(&place.home);
    assert!(
        running,
        "the server, the background command and the command: {entries:?}"
    );
    let mut owners: Vec<&str> = entries.iter().map(|(_, owner)| owner.as_str()).collect();
    owners.sort_unstable();
    assert_eq!(owners, ["server", "workspace", "workspace"]);
    killed.kill().unwrap();
    killed.wait().unwrap();
    let left: Vec<u32> = entries.iter().map(|(pid, _)| *pid).collect();
    assert!(
        left.iter().any(|pid| alive(*pid)),
        "a glave killed outright leaves its commands running: {left:?}"
    );
    let next = glave(&place, &model, &["serve", "--stdio"], "");
    assert!(next.status.success(), "{}", text(&next));
    let stopped = until(Duration::from_secs(10), || {
        !left.iter().any(|pid| alive(*pid))
    });
    assert!(
        stopped,
        "the next start stops everything the killed glave left: {left:?}"
    );
    assert!(recorded(&place.home).is_empty(), "and forgets it");
}
