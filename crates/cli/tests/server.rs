//! The person's server, the real binary (`specs/one-server.md`, "The person's server"): one per
//! glave folder, however many start at once; started from a terminal and outliving it; never
//! by a command with no one at the terminal; its socket refused when it is not the person's
//! own, and replaced, when stale, only by the lock's holder; out of a sandboxed command's
//! reach; and stopped gracefully. Every server a test starts is stopped when the test ends.

use std::io::{BufRead as _, BufReader};
use std::process::{Command, Output, Stdio};

use glave_ports::testing::Running;

use crate::support::{Place, place, text};

/// `glave` with `args` in `place`, with nothing from the person's own setup, still running.
fn start(place: &Place, args: &[&str]) -> Running {
    std::fs::create_dir_all(&place.home).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(args)
        .current_dir(&place.project)
        .env("GLAVE_HOME", &place.home)
        .env("NO_COLOR", "1")
        // No screen, so a link is printed rather than a browser opened.
        .env("SSH_CONNECTION", "test")
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_BASE_URL")
        .env_remove("GLAVE_PROVIDER")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    Running::new(child)
}

/// `glave` with `args`, to its end.
fn run(place: &Place, args: &[&str]) -> Output {
    start(place, args).wait_with_output().unwrap()
}

/// `glave` with `args` in a terminal of its own, which closes when it ends.
fn in_terminal(place: &Place, args: &[&str]) -> Running {
    let glave = env!("CARGO_BIN_EXE_glave");
    let spawn = "import pty, sys; sys.exit(pty.spawn(sys.argv[1:]) >> 8)";
    let python = [&["-c", spawn, glave][..], args].concat();
    std::fs::create_dir_all(&place.home).unwrap();
    let child = Command::new("python3")
        .args(python)
        .current_dir(&place.project)
        .env("GLAVE_HOME", &place.home)
        .env("SSH_CONNECTION", "test")
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_BASE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    Running::new(child)
}

/// The person's server in `place`, stopped when this drops, however the test ends.
struct Stopped<'a>(&'a Place);

impl Drop for Stopped<'_> {
    fn drop(&mut self) {
        let stopped = run(self.0, &["server", "stop"]);
        assert!(stopped.status.success(), "{}", text(&stopped));
    }
}

/// `glave server status`: its exit code and what it said.
fn status(place: &Place) -> (Option<i32>, String) {
    let said = run(place, &["server", "status"]);
    (said.status.code(), text(&said))
}

/// `glave serve` in the foreground on any free port, once it serves on its socket.
fn serve(place: &Place) -> Running {
    let mut served = start(place, &["serve", "--http", "127.0.0.1:0"]);
    let mut lines = BufReader::new(served.stderr.take().unwrap()).lines();
    loop {
        let line = lines.next().unwrap().unwrap();
        if line.starts_with("glave: serving you on ") {
            break;
        }
    }
    // Whatever it says later is still read, so it never blocks.
    std::thread::spawn(move || for _ in lines.map_while(Result::ok) {});
    served
}

fn cleaned(place: &Place) {
    let _ = std::fs::remove_dir_all(place.home.parent().unwrap());
}

#[test]
fn two_starts_at_once_make_one_server_which_outlives_their_terminals_and_stops() {
    let place = place("two");
    let stopped_at_end = Stopped(&place);
    let first = in_terminal(&place, &["app"]);
    let second = in_terminal(&place, &["app"]);
    for started in [first, second] {
        let said = started.wait_with_output().unwrap();
        assert!(said.status.success(), "{}", text(&said));
        assert!(text(&said).contains("#code="), "{}", text(&said));
    }
    // Both terminals are gone; the server they started is not.
    let (code, said) = status(&place);
    assert_eq!(code, Some(0), "{said}");
    assert!(said.contains("is running (process "), "{said}");
    let log = std::fs::read_to_string(place.home.join("run/server.log")).unwrap();
    assert_eq!(log.matches("glave: serving you on").count(), 1, "{log}");
    let stopped = run(&place, &["server", "stop"]);
    assert!(stopped.status.success(), "{}", text(&stopped));
    assert_eq!(status(&place).0, Some(3), "stopped");
    let log = std::fs::read_to_string(place.home.join("run/server.log")).unwrap();
    assert!(log.contains("glave: stopping"), "a graceful stop: {log}");
    assert!(!place.home.join("run/server.sock").exists(), "{log}");
    drop(stopped_at_end);
    cleaned(&place);
}

#[test]
fn no_command_without_a_terminal_starts_one() {
    let place = place("scripted");
    let refused = run(&place, &["app"]);
    assert!(!refused.status.success());
    assert!(
        text(&refused).contains("only glave app run in a terminal starts it"),
        "{}",
        text(&refused)
    );
    assert_eq!(status(&place).0, Some(3));
    assert!(!place.home.join("run/server.sock").exists());
    assert!(!place.home.join("run/server.log").exists());
    cleaned(&place);
}

#[test]
fn a_stale_socket_is_replaced_only_by_the_lock_s_holder() {
    let place = place("stale");
    let socket = place.home.join("run/server.sock");
    assert!(socket.as_os_str().len() <= 100, "{}", socket.display());
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        let private = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(&place.home, private.clone()).unwrap();
        std::fs::set_permissions(socket.parent().unwrap(), private).unwrap();
    }
    // A socket a crashed server left: no one listens.
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert_eq!(status(&place).0, Some(3));
    assert!(socket.exists(), "a client never removes it");
    let mut crashed = serve(&place);
    assert_eq!(status(&place).0, Some(0), "the holder replaced it");
    let second = run(&place, &["serve", "--http", "127.0.0.1:0"]);
    assert!(!second.status.success());
    assert!(text(&second).contains("already runs"), "{}", text(&second));
    assert_eq!(status(&place).0, Some(0), "the second left it alone");
    crashed.kill().unwrap();
    crashed.wait().unwrap();
    assert!(socket.exists(), "a crash leaves it");
    let next = serve(&place);
    assert_eq!(status(&place).0, Some(0), "the next holder replaced it");
    drop(next);
    cleaned(&place);
}

#[test]
fn a_socket_in_a_folder_others_may_open_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let place = place("open");
    let run_folder = place.home.join("run");
    let served = serve(&place);
    let open = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(&run_folder, open).unwrap();
    let (code, said) = status(&place);
    assert_ne!(code, Some(0));
    assert!(said.contains("is not your own glave server's"), "{said}");
    drop(served);
    let refused = run(&place, &["serve", "--http", "127.0.0.1:0"]);
    assert!(!refused.status.success());
    assert!(
        text(&refused).contains("is not private to you"),
        "{}",
        text(&refused)
    );
    cleaned(&place);
}

/// What a command the model runs in `place` with `sandbox` printed, through the in-process
/// glave: `None` when the sandbox cannot run here and no one requires it.
fn ran(place: &Place, sandbox: &str, script: &str) -> Option<String> {
    use glave_provider_openai::fake::{FakeServer, Reply};
    let call = serde_json::json!({ "command": script });
    let model = FakeServer::start(vec![
        Reply::tool_call("call_1", "bash", &call),
        Reply::text("Done."),
    ])
    .unwrap();
    let args = ["--model", "m", "--sandbox", sandbox, "--approval", "auto"];
    let said = crate::support::glave(place, &model, &[&args[..], &["go"]].concat(), "");
    let required = std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_some();
    if !said.status.success() && text(&said).contains("sandbox") && !required {
        return None;
    }
    assert!(said.status.success(), "{}", text(&said));
    let asked = model.received();
    Some(crate::support::said(&asked[1].body, "tool").concat())
}

#[test]
fn a_sandboxed_command_reaches_neither_the_socket_nor_the_token_or_keys() {
    let place = place("sandbox");
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(place.home.join("config.toml"), "titles = false\n").unwrap();
    let served = serve(&place);
    let (_, said) = status(&place);
    let socket = said
        .lines()
        .find_map(|line| line.trim().strip_prefix("socket: "))
        .unwrap()
        .to_owned();
    // Spelled so no approval rule stops it first: the sandbox alone must.
    let script = format!(
        "d='{}'; cat \"$d\"/serve.t* && echo token-read; cat \"$d\"/root.k* && echo key-read; \
             python3 -c \"import socket; s = socket.socket(socket.AF_UNIX); \
             s.connect('{socket}'); print('socket' + '-reached')\"; echo ran",
        place.home.display()
    );
    let Some(confined) = ran(&place, "project", &script) else {
        cleaned(&place);
        return;
    };
    assert!(confined.contains("ran"), "{confined}");
    for reached in ["token-read", "key-read", "socket-reached"] {
        assert!(!confined.contains(reached), "{reached}: {confined}");
    }
    // Control: unconfined, the same command reaches all three.
    let open = ran(&place, "off", &script).unwrap();
    for reached in ["token-read", "key-read", "socket-reached"] {
        assert!(open.contains(reached), "{reached}: {open}");
    }
    drop(served);
    cleaned(&place);
}
