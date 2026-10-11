//! Shared helpers for the binary's end-to-end tests: a throwaway project and home, and `glave`
//! run against a fake model server with nothing from the person's own setup.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};

use glave_provider_openai::fake::FakeServer;
use serde_json::Value;

/// A project folder and a glave home for one test, inside the system temp folder.
pub struct Place {
    pub project: PathBuf,
    pub home: PathBuf,
}

pub fn place(name: &str) -> Place {
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
pub fn glave(place: &Place, server: &FakeServer, args: &[&str], stdin: &str) -> Output {
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
pub fn start(place: &Place, server: &FakeServer, args: &[&str]) -> Child {
    start_with(place, server, args, &[])
}

/// `glave` started with `args` and the variables `env`, still running.
pub fn start_with(
    place: &Place,
    server: &FakeServer,
    args: &[&str],
    env: &[(&str, &str)],
) -> Child {
    Command::new(env!("CARGO_BIN_EXE_glave"))
        .args(args)
        .envs(env.iter().copied())
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

pub fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The text of every message of `role` in a request the model received.
pub fn said(body: &Value, role: &str) -> Vec<String> {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == role)
        .map(|message| message["content"].to_string())
        .collect()
}
