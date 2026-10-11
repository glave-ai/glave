//! A fresh workspace for one run of a task: its starting files, its environment and its setup
//! commands; and running the task's scripts there.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::error::EvalError;
use crate::task::Task;

/// The longest a setup command or script may take.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(600);

/// Lines of a failing script's output kept in its failure.
const LAST_LINES: usize = 8;

/// The environment commands run with: glave's own, then the run's, then the task's.
pub type Environment = BTreeMap<OsString, OsString>;

/// A workspace ready for the agent.
pub struct Prepared {
    pub workspace: PathBuf,
    pub environment: Environment,
}

/// Copies the task's files into `<dir>/workspace` and runs its setup commands there.
pub async fn prepare(
    base: &[(OsString, OsString)],
    task: &Task,
    dir: &Path,
) -> Result<Prepared, EvalError> {
    let workspace = dir.join("workspace");
    copy_files(task.environment(), workspace.clone()).await?;
    // `{path}` is the `PATH` commands get otherwise, so a task can put its own tools first.
    let path = base
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fill = |text: &str| {
        text.replace("{dir}", &dir.display().to_string())
            .replace("{workspace}", &workspace.display().to_string())
            .replace("{task}", &task.dir.display().to_string())
            .replace("{path}", &path)
    };
    let mut environment: Environment = base.iter().cloned().collect();
    // Each run builds in its own folder: runs at once would otherwise overwrite each other's
    // binaries. It sits outside the workspace and is removed once the run is graded.
    environment.insert("CARGO_TARGET_DIR".into(), target(dir, task).into());
    environment.insert("NO_COLOR".into(), "1".into());
    for (name, value) in &task.env {
        environment.insert(name.into(), fill(value).into());
    }
    for command in &task.setup {
        let command = fill(command);
        let output = script("sh", &["-c", &command], &workspace, &environment).await?;
        if !output.success {
            return Err(EvalError::Setup(format!(
                "{}: setup `{command}` failed: {}",
                task.name,
                last_lines(&output.text)
            )));
        }
    }
    Ok(Prepared {
        workspace,
        environment,
    })
}

/// Where the run in `dir` builds: in `dir`, or in the sandbox, which writes only the project
/// and temp folders, in a folder of the run's own in `/tmp`.
fn target(dir: &Path, task: &Task) -> PathBuf {
    match task.sandbox {
        Some(_) => {
            let mut hasher = std::hash::DefaultHasher::new();
            dir.hash(&mut hasher);
            Path::new("/tmp")
                .join(format!("glave-eval-{:016x}", hasher.finish()))
                .join("target")
        }
        None => dir.join("target"),
    }
}

/// Removes what a run built, keeping its workspace and records.
pub async fn tidy(dir: &Path, task: &Task) {
    // Best effort: a leftover build folder costs disk space, not correctness.
    let target = target(dir, task);
    let built = match task.sandbox {
        Some(_) => target.parent().unwrap_or(&target),
        None => &target,
    };
    let _ = tokio::fs::remove_dir_all(built).await;
}

/// A finished script.
pub struct Output {
    pub success: bool,
    /// Standard output and error, as text.
    pub text: String,
}

/// Runs a setup command or script in `dir`, outside any sandbox, without glave's endpoint
/// variables.
pub async fn script(
    program: &str,
    args: &[&str],
    dir: &Path,
    environment: &Environment,
) -> Result<Output, EvalError> {
    let child = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env_clear()
        .envs(
            environment
                .iter()
                .filter(|(name, _)| !name.to_string_lossy().starts_with("GLAVE_")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(EvalError::io("run a script in", dir))?;
    match tokio::time::timeout(SCRIPT_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => Ok(Output {
            success: output.status.success(),
            text: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        }),
        Ok(Err(err)) => Err(EvalError::io("wait for a script in", dir)(err)),
        Err(_) => Ok(Output {
            success: false,
            text: format!("stopped after {}s", SCRIPT_TIMEOUT.as_secs()),
        }),
    }
}

/// The end of a script's output, on one line.
pub fn last_lines(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let skip = lines.len().saturating_sub(LAST_LINES);
    lines
        .iter()
        .skip(skip)
        .copied()
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Copies the task's starting files, if it has any, into a new workspace.
async fn copy_files(from: PathBuf, to: PathBuf) -> Result<(), EvalError> {
    let copied = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&to).map_err(EvalError::io("create", &to))?;
        if from.is_dir() {
            copy_tree(&from, &to)?;
        }
        Ok(())
    })
    .await;
    copied.map_err(|err| EvalError::Setup(format!("copying the task's files stopped: {err}")))?
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), EvalError> {
    for entry in std::fs::read_dir(from).map_err(EvalError::io("list", from))? {
        let entry = entry.map_err(EvalError::io("list", from))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if source.is_dir() {
            std::fs::create_dir_all(&target).map_err(EvalError::io("create", &target))?;
            copy_tree(&source, &target)?;
        } else {
            std::fs::copy(&source, &target).map_err(EvalError::io("copy", &source))?;
        }
    }
    Ok(())
}
