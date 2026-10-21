//! glave never leaves an orphaned process (#361, #558), end to end. A glave killed outright
//! mid-turn, with a command running, one kept in the background and an MCP server that started
//! a program of its own, has every one of them, and all they started, stopped at once by its
//! lifeline's helper, sandboxed or not. When the helper goes too, the ledger still holds them,
//! and the next glave to start stops every one.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use glave_ports::testing::{HUNG, Running};
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

/// The children of `parent` that `pgrep` with `filter` finds.
fn children(parent: u32, filter: &[&str]) -> Vec<u32> {
    let listed = Command::new("pgrep")
        .args(["-P", &parent.to_string()])
        .args(filter)
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .split_whitespace()
        .map(|pid| pid.parse().unwrap())
        .collect()
}

/// Everything `root` started, and what those started, down to the last.
fn descendants(root: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let mut next = vec![root];
    while let Some(parent) = next.pop() {
        let below = children(parent, &[]);
        next.extend(&below);
        found.extend(below);
    }
    found
}

/// glave's lifeline helper: the child of `glave` started as one.
fn helper_of(glave: u32) -> Option<u32> {
    children(glave, &["-f", "glave-lifeline-1"])
        .first()
        .copied()
}

/// What each of `pids` runs, as `ps` shows it.
fn commands(pids: &[u32]) -> Vec<String> {
    let listed: Vec<String> = pids.iter().map(u32::to_string).collect();
    let shown = Command::new("ps")
        .args(["-o", "command=", "-p", &listed.join(",")])
        .output()
        .unwrap();
    String::from_utf8_lossy(&shown.stdout)
        .lines()
        .map(|line| line.trim().to_owned())
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

/// glave in `place`, mid-turn, in `sandbox` (glave's default when none): a command kept in the
/// background and one in the foreground, each with a program of its own beside it, and an MCP
/// server, which `server_starts` one too; and every process glave started and those started.
fn mid_turn(
    place: &Place,
    sandbox: Option<&str>,
    server_starts: bool,
) -> (Running, FakeServer, Vec<u32>) {
    choose(place, "memory", "\"off\"");
    if let Some(sandbox) = sandbox {
        choose(place, "sandbox", sandbox);
    }
    let beside = if server_starts { "sleep 300 & " } else { "" };
    let server = format!("{beside}exec '{}'", test_server().display());
    let servers = json!({ "mcpServers": { "tests": { "command": "sh", "args": ["-c", server] } } });
    std::fs::write(place.home.join("mcp.json"), servers.to_string()).unwrap();
    let model = FakeServer::start(vec![
        Reply::tool_call(
            "call_1",
            "bash",
            &json!({"command": "sleep 300 & sleep 302", "background": true}),
        ),
        Reply::tool_call(
            "call_2",
            "bash",
            &json!({"command": "sleep 303 & sleep 301"}),
        ),
        Reply::text("never reached"),
    ])
    .unwrap();
    let args = ["--model", "m", "--approval", "auto", "go"];
    let glave = start(place, &model, &args);
    let running = until(HUNG, || recorded(&place.home).len() == 3);
    let entries = recorded(&place.home);
    assert!(
        running,
        "the server, the background command and the command: {entries:?}"
    );
    let mut owners: Vec<&str> = entries.iter().map(|(_, owner)| owner.as_str()).collect();
    owners.sort_unstable();
    assert_eq!(owners, ["server", "workspace", "workspace"]);
    // The helper, the server and the commands, and each program started beside them.
    let mut beside = vec!["sleep 300", "sleep 301", "sleep 302", "sleep 303"];
    if server_starts {
        beside.push("sleep 300");
    }
    beside.sort_unstable();
    let pid = glave.id();
    let sleeping = || {
        let mut sleeps: Vec<String> = commands(&descendants(pid))
            .into_iter()
            .filter(|command| command.starts_with("sleep "))
            .collect();
        sleeps.sort_unstable();
        sleeps
    };
    let started = until(HUNG, || sleeping() == beside);
    assert!(started, "{:?}", commands(&descendants(pid)));
    let every = descendants(pid);
    (glave, model, every)
}

fn kill_outright(glave: &mut Running) {
    glave.kill().unwrap();
    glave.wait().unwrap();
}

fn killed_mid_turn_leaves_nothing(name: &str, sandbox: Option<&str>) {
    // Where no sandbox can run, a sandboxed command asks, and no one is here to answer: nothing
    // would run to prove anything. Where CI requires one, it runs.
    let required = ["GLAVE_REQUIRE_SANDBOX", "GLAVE_REQUIRE_LANDLOCK"]
        .iter()
        .any(|variable| std::env::var_os(variable).is_some());
    if sandbox != Some("\"off\"") && !glave_computer::sandbox_available() && !required {
        return;
    }
    let place = place(name);
    let (mut killed, _model, every) = mid_turn(&place, sandbox, true);
    let _folders = TempFolders(killed.id());
    kill_outright(&mut killed);
    let stopped = until(HUNG, || !every.iter().any(|pid| alive(*pid)));
    let left: Vec<&u32> = every.iter().filter(|pid| alive(**pid)).collect();
    assert!(stopped, "nothing glave started outlives it: {left:?}");
}

/// The temp folders the sandboxed commands of the glave it names had, which a glave killed
/// outright cannot remove: removed when this drops, however the test ends. macOS keeps them
/// in the system's temp folder, Linux in each user's sandbox folder there.
struct TempFolders(u32);

impl Drop for TempFolders {
    fn drop(&mut self) {
        let temp = std::env::temp_dir();
        let uid = rustix::process::getuid().as_raw();
        let places = [
            (temp.clone(), format!("glave-sandbox-{}-", self.0)),
            (
                temp.join(format!("glave-sandbox-{uid}")),
                format!("{}-", self.0),
            ),
        ];
        for (folder, start) in places {
            let Ok(listed) = std::fs::read_dir(folder) else {
                continue;
            };
            for entry in listed.filter_map(Result::ok) {
                if entry.file_name().to_string_lossy().starts_with(&start) {
                    let _ = glave_ports::remove_tree(&entry.path());
                }
            }
        }
    }
}

#[test]
fn a_glave_killed_outright_has_what_it_started_stopped_at_once() {
    // Unconfined, so nothing but the lifeline stops what it leaves.
    killed_mid_turn_leaves_nothing("orphans", Some("\"off\""));
}

#[test]
fn a_glave_killed_outright_has_its_sandboxed_commands_stopped_at_once() {
    killed_mid_turn_leaves_nothing("sandboxed", None);
}

#[test]
fn a_glave_killed_outright_has_its_landlocked_commands_stopped_at_once() {
    // Read-only, the level Landlock keeps where bubblewrap cannot start (CI's Landlock job):
    // there each command runs under glave's own sandbox helper, in a session of its own.
    killed_mid_turn_leaves_nothing("landlocked", Some("\"read-only\""));
}

#[test]
fn when_glave_and_its_helper_are_killed_the_next_start_stops_what_they_left() {
    let place = place("helperless");
    // A server that starts nothing beside it: once its leader ends, as a server does when glave
    // goes, nothing proves the rest of its group is still the one recorded.
    let (mut killed, model, every) = mid_turn(&place, Some("\"off\""), false);
    let helper = helper_of(killed.id()).expect("glave's lifeline helper runs");
    Command::new("kill")
        .args(["-KILL", &helper.to_string()])
        .status()
        .unwrap();
    kill_outright(&mut killed);
    assert!(
        every.iter().any(|pid| alive(*pid)),
        "a glave killed with its helper leaves its commands running: {every:?}"
    );
    let next = glave(&place, &model, &["serve", "--stdio"], "");
    assert!(next.status.success(), "{}", text(&next));
    let stopped = until(HUNG, || !every.iter().any(|pid| alive(*pid)));
    let left: Vec<&u32> = every.iter().filter(|pid| alive(**pid)).collect();
    assert!(
        stopped,
        "the next start stops everything they left: {left:?}"
    );
    assert!(recorded(&place.home).is_empty(), "and forgets it");
}
