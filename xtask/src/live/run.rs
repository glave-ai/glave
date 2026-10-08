//! Running glave on one task: a fresh project and home, and each turn as its own process, the
//! way a person or a server sends the next message.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::check::Context;
use super::tasks::{Task, Turn};
use super::trace::{self, Trace};

/// What every task runs with.
pub struct Setup {
    pub binary: PathBuf,
    pub model: String,
    /// Where commands in the projects build, shared so builds are cached across tasks.
    pub cargo_target: PathBuf,
    /// The longest one turn may take.
    pub timeout: Duration,
}

/// How a task went.
pub struct Report {
    pub name: &'static str,
    pub trace: Trace,
    pub elapsed: Duration,
    /// Empty when the task passed.
    pub failures: Vec<String>,
}

/// Runs `task` in `dir`, keeping its project, home, events and transcript there.
pub fn task(setup: &Setup, task: &Task, dir: &Path) -> Result<Report, String> {
    let project = dir.join("project");
    write_files(&project, task.files)?;
    let events = dir.join("events.jsonl");
    let transcript = dir.join("transcript.txt");
    let output = File::create(&transcript)
        .map_err(|err| format!("could not create {}: {err}", transcript.display()))?;
    let start = Instant::now();
    let mut failures = Vec::new();
    for (index, turn) in task.turns.iter().enumerate() {
        let mut child = spawn(setup, turn, index > 0, &project, dir, &output)?;
        match wait(&mut child, setup.timeout)? {
            Some(status) if status.success() => {}
            Some(status) => failures.push(format!("turn {} exited with {status}", index + 1)),
            None => {
                failures.push(format!(
                    "turn {} took longer than {}s",
                    index + 1,
                    setup.timeout.as_secs()
                ));
                break;
            }
        }
    }
    let elapsed = start.elapsed();
    let trace = trace::load(&events).unwrap_or_else(|err| {
        failures.push(err);
        Trace::default()
    });
    let cx = Context {
        trace: &trace,
        project: &project,
        model: &setup.model,
        files: task.files,
        cargo_target: &setup.cargo_target,
    };
    failures.extend(
        task.checks
            .iter()
            .filter_map(|check| check.judge(&cx).err()),
    );
    Ok(Report {
        name: task.name,
        trace,
        elapsed,
        failures,
    })
}

fn write_files(project: &Path, files: &[(&str, &str)]) -> Result<(), String> {
    std::fs::create_dir_all(project)
        .map_err(|err| format!("could not create {}: {err}", project.display()))?;
    for (name, contents) in files {
        let path = project.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
        }
        std::fs::write(&path, contents)
            .map_err(|err| format!("could not write {}: {err}", path.display()))?;
    }
    Ok(())
}

/// Starts one turn. No one is at the keyboard: standard input is empty, so any request to act
/// is declined unless the turn allows it.
fn spawn(
    setup: &Setup,
    turn: &Turn,
    continuing: bool,
    project: &Path,
    dir: &Path,
    output: &File,
) -> Result<Child, String> {
    let copy = || {
        output
            .try_clone()
            .map_err(|err| format!("could not share the transcript: {err}"))
    };
    let mut command = Command::new(&setup.binary);
    command
        .arg("--model")
        .arg(&setup.model)
        .arg("--events")
        .arg(dir.join("events.jsonl"));
    if turn.allow {
        command.arg("--yes");
    }
    if continuing {
        command.arg("--continue");
    }
    command
        .arg("--")
        .arg(turn.prompt)
        .current_dir(project)
        .env("GLAVE_HOME", dir.join("home"))
        .env("NO_COLOR", "1")
        .env("CARGO_TARGET_DIR", &setup.cargo_target)
        .stdin(Stdio::null())
        .stdout(copy()?)
        .stderr(copy()?)
        .spawn()
        .map_err(|err| format!("could not start {}: {err}", setup.binary.display()))
}

/// Waits for the turn, stopping it after `limit`. `None` means it was stopped.
fn wait(child: &mut Child, limit: Duration) -> Result<Option<ExitStatus>, String> {
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("could not wait for glave: {err}"))?
        {
            return Ok(Some(status));
        }
        if start.elapsed() >= limit {
            // Best effort: the turn is reported as too slow either way.
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
