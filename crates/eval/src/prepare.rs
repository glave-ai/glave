//! A fresh workspace for one run of a task: its starting files, its environment and its setup
//! commands; and running the task's scripts there.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use glave_process::{Processes, Program};

use crate::error::EvalError;
use crate::task::Task;

/// The longest a setup command or script may take.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(600);

/// Lines of a failing script's output kept in its failure.
const LAST_LINES: usize = 8;

/// A run's own temp folder, in its folder.
const TEMP: &str = "tmp";

/// The environment commands run with: glave's own, then the run's, then the task's.
pub(crate) type Environment = BTreeMap<OsString, OsString>;

/// A run's own Cargo home, in its folder.
const CARGO_HOME: &str = "cargo-home";

/// The folders of this machine's Cargo home that hold what it downloaded: the crates and their
/// index, and git checkouts.
const CARGO_DOWNLOADS: [&str; 2] = ["registry", "git"];

/// A workspace ready for the agent.
pub(crate) struct Prepared {
    pub workspace: PathBuf,
    pub environment: Environment,
    /// What its commands may not read: this machine's crate downloads, which a fresh computer
    /// would not have.
    pub hidden: Vec<PathBuf>,
}

/// Copies the task's files into `<dir>/workspace` and runs its setup commands there.
pub(crate) async fn prepare(
    processes: &Processes,
    base: &[(OsString, OsString)],
    task: &Task,
    dir: &Path,
) -> Result<Prepared, EvalError> {
    let workspace = dir.join("workspace");
    copy_files(task.environment(), workspace.clone()).await?;
    // `{path}` is the `PATH` commands get otherwise, so a task can put its own tools first.
    let path = variable(base, "PATH")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fill = |text: &str| {
        text.replace("{dir}", &dir.display().to_string())
            .replace("{workspace}", &workspace.display().to_string())
            .replace("{task}", &task.dir.display().to_string())
            .replace("{path}", &path)
    };
    let mut environment: Environment = base.iter().cloned().collect();
    // Each run has a temp folder of its own, made empty, for what runs outside the sandbox,
    // its setup and its grader (the sandbox gives each command one): runs at once would
    // otherwise share the machine's, and leave there what they wrote. Removed once graded.
    let temp = dir.join(TEMP);
    tokio::fs::create_dir_all(&temp)
        .await
        .map_err(EvalError::io("create", &temp))?;
    environment.insert("TMPDIR".into(), temp.into());
    // Each run builds in its own folder: runs at once would otherwise overwrite each other's
    // binaries. It sits outside the workspace and is removed once the run is graded.
    environment.insert("CARGO_TARGET_DIR".into(), target(dir, task).into());
    // It starts as a fresh computer would, with an empty Cargo home of its own, and this
    // machine's crate downloads hidden from its commands (`hidden`), which could otherwise
    // build offline what a fresh computer would have to download. Removed with the build.
    let cargo_home = dir.join(CARGO_HOME);
    tokio::fs::create_dir_all(&cargo_home)
        .await
        .map_err(EvalError::io("create", &cargo_home))?;
    environment.insert("CARGO_HOME".into(), cargo_home.into());
    environment.insert("NO_COLOR".into(), "1".into());
    for (name, value) in &task.env {
        environment.insert(name.into(), fill(value).into());
    }
    for command in &task.setup {
        let command = fill(command);
        let output = script(processes, "sh", &["-c", &command], &workspace, &environment).await?;
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
        hidden: cargo_downloads(base),
    })
}

/// The value of `name` in `environment`, when it is set.
fn variable<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    (environment.iter())
        .find(|(key, value)| key == name && !value.is_empty())
        .map(|(_, value)| value)
}

/// Where this machine's Cargo home keeps what it downloaded, as `cargo` finds that home in
/// `environment`: `CARGO_HOME`, else `~/.cargo`.
fn cargo_downloads(environment: &[(OsString, OsString)]) -> Vec<PathBuf> {
    let home = variable(environment, "CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| variable(environment, "HOME").map(|home| Path::new(home).join(".cargo")));
    (home.iter())
        .flat_map(|home| CARGO_DOWNLOADS.iter().map(|folder| home.join(folder)))
        .collect()
}

/// Whether the task's commands run in the sandbox wherever it can run: below full access.
fn sandboxed(task: &Task) -> bool {
    task.permissions != glave_harness::Permissions::FullAccess
}

/// Where the run in `dir` builds: in `dir`, or in the sandbox, which writes only the project
/// and temp folders, in a folder of the run's own in `/tmp`.
fn target(dir: &Path, task: &Task) -> PathBuf {
    if !sandboxed(task) {
        return dir.join("target");
    }
    let mut hasher = std::hash::DefaultHasher::new();
    dir.hash(&mut hasher);
    Path::new("/tmp")
        .join(format!("glave-eval-{:016x}", hasher.finish()))
        .join("target")
}

/// Removes what a run built and downloaded, keeping its workspace and records.
pub(crate) async fn tidy(dir: &Path, task: &Task) {
    // Its temp folder too, with whatever its scripts left there, read-only folders and all.
    let temp = dir.join(TEMP);
    let _ = tokio::task::spawn_blocking(move || glave_ports::remove_tree(&temp)).await;
    // Best effort: a leftover build folder costs disk space, not correctness.
    let target = target(dir, task);
    let built = if sandboxed(task) {
        target.parent().unwrap_or(&target)
    } else {
        &target
    };
    let _ = tokio::fs::remove_dir_all(built).await;
    let _ = tokio::fs::remove_dir_all(dir.join(CARGO_HOME)).await;
}

/// A finished script.
pub(crate) struct Output {
    pub success: bool,
    /// Standard output and error, as text.
    pub text: String,
}

/// Runs a setup command or script in `dir`, outside any sandbox, without glave's endpoint
/// variables: one of `processes`, so it stops when the eval does, however it ends.
pub(crate) async fn script(
    processes: &Processes,
    program: &str,
    args: &[&str],
    dir: &Path,
    environment: &Environment,
) -> Result<Output, EvalError> {
    let mut script = Program::new(program);
    script
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
        .stderr(Stdio::piped());
    match processes.output(&mut script, SCRIPT_TIMEOUT).await {
        Ok(output) => Ok(Output {
            success: output.status.success(),
            text: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        }),
        Err(glave_process::ProcessError::TimedOut { .. }) => Ok(Output {
            success: false,
            text: format!(
                "stopped: it did not finish within {}",
                glave_ports::spoken(SCRIPT_TIMEOUT)
            ),
        }),
        Err(err) => Err(EvalError::io("run a script in", dir)(
            std::io::Error::other(err),
        )),
    }
}

/// The end of a script's output, on one line.
pub(crate) fn last_lines(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        (pairs.iter())
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect()
    }

    /// The downloads hidden are in the Cargo home `cargo` itself would use.
    #[test]
    fn this_machine_s_downloads_are_where_cargo_keeps_them() {
        let named = environment(&[("HOME", "/h"), ("CARGO_HOME", "/c")]);
        assert_eq!(
            cargo_downloads(&named),
            [PathBuf::from("/c/registry"), PathBuf::from("/c/git")]
        );
        let unnamed = environment(&[("HOME", "/h"), ("CARGO_HOME", "")]);
        assert_eq!(
            cargo_downloads(&unnamed),
            [
                PathBuf::from("/h/.cargo/registry"),
                PathBuf::from("/h/.cargo/git")
            ]
        );
        assert_eq!(cargo_downloads(&environment(&[])), Vec::<PathBuf>::new());
    }
}
