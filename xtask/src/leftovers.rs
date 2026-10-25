//! What the test suite leaves behind. `cargo xtask ci` runs the tests with a temp folder of
//! their own (`TMPDIR`); afterwards no process started with it may still run, and no `glave-*`
//! entry may remain in it but those glave keeps there on purpose. A leftover fails the run,
//! listed; the processes are stopped and the folder removed either way.

use std::path::{Path, PathBuf};
use std::time::Duration;

use glave_process::Program;

/// How long `ps` and `kill` have to answer, which they do at once.
const ANSWERS_WITHIN: Duration = Duration::from_secs(30);

/// What glave keeps in the temp folder on purpose, and why: an entry's name, or the start of
/// it followed by digits only.
const KEPT: &[(&str, &str)] = &[
    (
        "glave-workspaces",
        "copies of workspaces opened with no folder of their own go here",
    ),
    ("glave-eval", "eval runs keep their folders here to be read"),
    (
        "glave-sandbox-",
        "on Linux, each user's private folder that every sandbox shares (`glave-sandbox-<uid>`)",
    ),
];

/// The tests' own temp folder, made fresh for one run.
#[derive(Debug)]
pub(crate) struct Run {
    folder: PathBuf,
}

impl Run {
    /// A new, private folder for this run. Under `/tmp` where there is one: its path is short,
    /// so the sockets tests make in it stay under the length a socket's path may have.
    pub(crate) fn start() -> Result<Self, String> {
        let base = if Path::new("/tmp").is_dir() {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let folder = base.join(format!("glave-ci-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&folder)
            .map_err(|err| format!("could not make {}: {err}", folder.display()))?;
        Ok(Self { folder })
    }

    /// The variables that give a command this folder as its temp folder.
    pub(crate) fn env(&self) -> Vec<(&'static str, String)> {
        vec![("TMPDIR", self.folder.display().to_string())]
    }

    /// Fails, listing them, if a process started with this folder still runs or a `glave-*`
    /// entry glave does not keep is left in it.
    pub(crate) fn check(self) -> Result<(), String> {
        let processes = running(&self.folder)?;
        for (pid, _) in &processes {
            let _ = Program::new("kill")
                .args(["-KILL", pid])
                .output_blocking(ANSWERS_WITHIN);
        }
        let entries = left(&self.folder)?;
        let _ = std::fs::remove_dir_all(&self.folder);
        if processes.is_empty() && entries.is_empty() {
            println!("xtask: the tests left no process running and nothing in the temp folder");
            return Ok(());
        }
        let mut said = Vec::new();
        if !processes.is_empty() {
            let listed: Vec<String> = processes
                .iter()
                .map(|(pid, command)| format!("  {pid} {command}"))
                .collect();
            said.push(format!(
                "{} process(es) the tests started still ran (now stopped); every test stops \
                 what it starts:\n{}",
                processes.len(),
                listed.join("\n")
            ));
        }
        if !entries.is_empty() {
            said.push(format!(
                "the tests left {} entr(ies) in the temp folder; every test removes what it \
                 makes (glave_ports::testing::TempDir):\n  {}",
                entries.len(),
                entries.join("\n  ")
            ));
        }
        Err(said.join("\n"))
    }
}

/// The processes whose environment gives `folder` as their temp folder: pid and command
/// (never the environment, which may hold secrets).
fn running(folder: &Path) -> Result<Vec<(String, String)>, String> {
    let marker = format!("TMPDIR={}", folder.display());
    let pids = started_with(&ps(&["axeww", "-o", "pid=,command="])?, &marker);
    pids.into_iter()
        .map(|pid| {
            let command = ps(&["-o", "command=", "-p", &pid])?;
            let command: String = command.trim().chars().take(160).collect();
            Ok((pid, command))
        })
        .collect()
}

/// What `ps` with `args` lists.
fn ps(args: &[&str]) -> Result<String, String> {
    let output = Program::new("ps")
        .args(args)
        .output_blocking(ANSWERS_WITHIN)
        .map_err(|err| format!("could not list processes with `ps`: {err}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The pids of the lines of `ps` output naming `marker` as a whole word.
fn started_with(listed: &str, marker: &str) -> Vec<String> {
    listed
        .lines()
        .filter(|line| line.split_whitespace().any(|word| word == marker))
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect()
}

/// The `glave-*` entries in `folder` glave does not keep, sorted.
fn left(folder: &Path) -> Result<Vec<String>, String> {
    let listed = std::fs::read_dir(folder)
        .map_err(|err| format!("could not list {}: {err}", folder.display()))?;
    let mut left: Vec<String> = listed
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("glave-") && kept(name).is_none())
        .collect();
    left.sort();
    Ok(left)
}

/// Why glave keeps `name` in the temp folder, if it does.
fn kept(name: &str) -> Option<&'static str> {
    KEPT.iter().find_map(|(kept, why)| {
        let rest = name.strip_prefix(kept)?;
        let matches = if kept.ends_with('-') {
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        } else {
            rest.is_empty()
        };
        matches.then_some(*why)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_glave_keeps_is_allowed() {
        assert!(kept("glave-workspaces").is_some());
        assert!(kept("glave-eval").is_some());
        assert!(kept("glave-sandbox-501").is_some());
        assert!(
            kept("glave-sandbox-501-0").is_none(),
            "a command's own folder"
        );
        assert!(kept("glave-sandbox-").is_none());
        assert!(kept("glave-evals").is_none());
        assert!(kept("glave-session-x-1-0").is_none());
    }

    #[test]
    fn processes_are_found_by_their_temp_folder_alone() {
        let listed = "  12 /bin/glave serve TMPDIR=/tmp/glave-ci-7 HOME=/h\n\
                      13 /bin/glave serve TMPDIR=/tmp/glave-ci-70\n\
                      14 /bin/zsh HOME=/h\n";
        assert_eq!(started_with(listed, "TMPDIR=/tmp/glave-ci-7"), ["12"]);
    }
}
