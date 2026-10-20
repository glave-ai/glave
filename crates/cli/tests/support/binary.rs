//! Shared helpers for the binary's end-to-end tests: a throwaway project and home, and `glave`
//! run against a fake model server with nothing from the person's own setup.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use glave_ports::testing::{FAKE_SECRET, Running, TempDir};
use glave_provider_openai::fake::FakeServer;
use serde_json::Value;

/// A project folder and a glave home for one test, inside the system temp folder.
pub(crate) struct Place {
    pub project: PathBuf,
    pub home: PathBuf,
    /// The folder both are in, removed when the place drops; none for a place made of
    /// another's folders.
    pub _folder: Option<TempDir>,
}

pub(crate) fn place(name: &str) -> Place {
    // Named without a prefix: a longer path would be too long for its server's socket.
    let dir = TempDir::new(name).unwrap();
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("README.md"), "# demo\n").unwrap();
    Place {
        project,
        home: dir.join("home"),
        _folder: Some(dir),
    }
}

/// `glave` with `args`, talking only to `server`, with nothing from the person's own setup.
pub(crate) fn glave(place: &Place, server: &FakeServer, args: &[&str], stdin: &str) -> Output {
    let mut child = start(place, server, args);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// `glave` started with `args`, still running, and stopped when dropped.
pub(crate) fn start(place: &Place, server: &FakeServer, args: &[&str]) -> Running {
    start_with(place, server, args, &[])
}

/// `glave` started with `args` and the variables `env`, still running, and stopped when
/// dropped.
pub(crate) fn start_with(
    place: &Place,
    server: &FakeServer,
    args: &[&str],
    env: &[(&str, &str)],
) -> Running {
    let child = Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(args)
        .envs(env.iter().copied())
        .current_dir(&place.project)
        .env("GLAVE_BASE_URL", server.url())
        .env("GLAVE_HOME", &place.home)
        .env("GLAVE_API_KEY", FAKE_SECRET)
        .env("NO_COLOR", "1")
        .env_remove("GLAVE_MODEL")
        .env_remove("GLAVE_PROVIDER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    Running::new(child)
}

pub(crate) fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The text of every message of `role` in a request the model received.
pub(crate) fn said(body: &Value, role: &str) -> Vec<String> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == role)
        .map(|message| message["content"].to_string())
        .collect()
}
