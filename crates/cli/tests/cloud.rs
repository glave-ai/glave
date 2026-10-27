//! `glave --on cloud` end to end, against a real Docker Engine and a real worker: only with
//! `GLAVE_TEST_DOCKER=1`, `GLAVE_TEST_WORKER_GLAVE` (a Linux glave for the engine's processor;
//! CI's Linux job passes the one it just built) and `GLAVE_TEST_WORKER_IMAGE` (an image with
//! bash and git, pinned by digest). Elsewhere it says it skipped.
//!
//! The project's origin is served by `git daemon` with pushes allowed, at an address the
//! worker reaches (the Docker bridge's gateway on Linux, `host.docker.internal` on Docker
//! Desktop). The model writes a file on the worker, the session ends, the worker is released,
//! and its unpushed work arrives in origin on a `glave/wip/` branch.

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use glave_ports::testing::{Running, TempDir};
use glave_provider_openai::fake::{FakeServer, Reply};
use serde_json::json;

use crate::support::*;

fn run(dir: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The worker containers there are now.
fn workers() -> Vec<String> {
    let listed = run(
        Path::new("."),
        "docker",
        &["ps", "-aq", "--filter", "label=glave.worker"],
    );
    listed.lines().map(str::to_owned).collect()
}

/// Where a container reaches this machine.
fn host() -> String {
    if cfg!(target_os = "linux") {
        run(
            Path::new("."),
            "docker",
            &[
                "network",
                "inspect",
                "bridge",
                "--format",
                "{{(index .IPAM.Config 0).Gateway}}",
            ],
        )
    } else {
        "host.docker.internal".to_owned()
    }
}

/// `git daemon` serving `base` on all addresses, pushes allowed, once it listens; stopped
/// when dropped, with the pipe it logs to, which it needs open while it runs.
struct Daemon {
    _running: Running,
    _log: ChildStderr,
}

/// A [`Daemon`] on a free port, and that port. A port is free only until something takes it:
/// another test's server may bind the one found before the daemon does, so a daemon that
/// could not bind its port ends, and the next try takes another.
fn daemon(base: &Path) -> (Daemon, u16) {
    for _ in 0..10 {
        let port = std::net::TcpListener::bind("0.0.0.0:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if let Some(daemon) = daemon_on(base, port) {
            return (daemon, port);
        }
    }
    panic!("git daemon did not start");
}

/// A [`Daemon`] on `port` once it says it listens, or none when it ends first, unable to bind
/// `port`. Never probed by connecting, which would reach whatever else took the port instead.
/// Started as `git-daemon` itself: `git daemon` runs it as a child of its own, which stopping
/// `git` would leave running.
fn daemon_on(base: &Path, port: u16) -> Option<Daemon> {
    let exec_path = Command::new("git").arg("--exec-path").output().unwrap();
    let exec_path = String::from_utf8(exec_path.stdout).unwrap();
    let child = Command::new(Path::new(exec_path.trim()).join("git-daemon"))
        .args([
            "--verbose",
            "--export-all",
            "--enable=receive-pack",
            "--listen=0.0.0.0",
            &format!("--port={port}"),
            &format!("--base-path={}", base.display()),
            &base.display().to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut running = Running::new(child);
    let log = running.stderr.take()?;
    // Its log read on a thread until it says it listens, then handed back to stay open; a
    // daemon that ends first closes it instead.
    let (sender, said) = mpsc::channel();
    std::thread::spawn(move || {
        let mut lines = BufReader::new(log);
        let mut line = String::new();
        while lines.read_line(&mut line).is_ok_and(|read| read > 0) {
            if line.contains("Ready to rumble") {
                let _ = sender.send(lines.into_inner());
                return;
            }
            line.clear();
        }
    });
    match said.recv_timeout(Duration::from_secs(10)) {
        Ok(log) => Some(Daemon {
            _running: running,
            _log: log,
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("git daemon said nothing in 10 s"),
    }
}

#[test]
fn a_daemon_whose_port_another_server_took_ends_rather_than_share_it() {
    let base = TempDir::new("git-daemon").unwrap();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    assert!(daemon_on(&base, port).is_none());
    drop(taken);
    let (_daemon, _) = daemon(&base);
}

/// `project` as a repository with one commit, pushed to `origin.git` in a folder beside
/// it: that folder, and the bare repository.
fn pushed_project(project: &Path) -> (PathBuf, PathBuf) {
    let repos = project.with_file_name("repos");
    std::fs::create_dir_all(&repos).unwrap();
    let origin = repos.join("origin.git");
    let bare = origin.display().to_string();
    run(&repos, "git", &["init", "-q", "--bare", &bare]);
    run(project, "git", &["init", "-q", "--initial-branch=main"]);
    run(project, "git", &["add", "README.md"]);
    let who = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
    let commit = [&who[..], &["commit", "-q", "-m", "one"]].concat();
    run(project, "git", &commit);
    run(project, "git", &["remote", "add", "origin", &bare]);
    run(project, "git", &["push", "-q", "-u", "origin", "main"]);
    (repos, origin)
}

#[test]
fn a_thread_on_a_worker_pushes_its_work_when_released() {
    let wanted = |name| std::env::var(name).ok().filter(|value| !value.is_empty());
    let (Some(_), Some(linux_glave), Some(image)) = (
        wanted("GLAVE_TEST_DOCKER").filter(|on| on == "1"),
        wanted("GLAVE_TEST_WORKER_GLAVE"),
        wanted("GLAVE_TEST_WORKER_IMAGE"),
    ) else {
        eprintln!(
            "skipped: set GLAVE_TEST_DOCKER=1, GLAVE_TEST_WORKER_GLAVE (a Linux glave) and \
                 GLAVE_TEST_WORKER_IMAGE (an image with bash and git, pinned) for --on cloud"
        );
        return;
    };
    let place = place("cloud");
    let (repos, origin) = pushed_project(&place.project);
    let project = &place.project;
    let (daemon, port) = daemon(&repos);
    let url = format!("git://{}:{port}/origin.git", host());
    run(project, "git", &["remote", "set-url", "origin", &url]);
    std::fs::create_dir_all(project.join(".glave")).unwrap();
    std::fs::write(
        project.join(".glave/environment.toml"),
        format!("image = \"{image}\"\n"),
    )
    .unwrap();
    std::fs::create_dir_all(&place.home).unwrap();
    std::fs::write(
        place.home.join("config.toml"),
        format!("on = \"cloud\"\n\n[workers]\nbackend = \"docker\"\nglave = \"{linux_glave}\"\n"),
    )
    .unwrap();

    let before = workers();
    let server = FakeServer::start(vec![
        Reply::tool_call(
            "toolu_1",
            "write",
            &json!({"path": "notes.md", "content": "from the worker\n"}),
        ),
        Reply::text("I wrote notes.md."),
    ])
    .unwrap();
    let output = glave(&place, &server, &["--model", "m", "write notes"], "");
    drop(daemon);
    let shown = text(&output);
    assert!(output.status.success(), "{shown}");
    assert!(shown.contains("glave/wip/"), "{shown}");
    assert!(
        !project.join("notes.md").exists(),
        "the work happened on the worker"
    );
    let wip = run(
        &origin,
        "git",
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/glave/wip",
        ],
    );
    assert_eq!(wip.lines().count(), 1, "{wip}");
    let pushed = run(&origin, "git", &["show", &format!("{wip}:notes.md")]);
    assert_eq!(pushed, "from the worker");
    let sent = said(&server.received()[0].body, "user").join("\n");
    assert!(sent.contains("cloud worker"), "{sent}");
    assert_eq!(workers(), before, "the worker is removed");
}
