//! Work moves between computers only through git: a repository is cloned into a workspace,
//! and a save commits everything to a branch and pushes it back. A plain folder keeps a
//! hidden history of its own, capped in size, and never becomes a repository itself.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;

use glave_ports::{ComputerError, GitCredential, Save, Saved};

use super::files::total_size;
use super::registry::Opened;

/// Where a plain folder's history lives, inside the folder.
pub(crate) const HISTORY: &str = ".glave/history.git";
/// The same history, as a line in `.glave/.gitignore`, which lives inside `.glave`.
const IGNORED: &str = "history.git";
/// The largest plain folder whose history glave keeps; past this, make it a repository.
const MAX_PLAIN_BYTES: u64 = 100 * 1024 * 1024;

/// One git invocation: the environment it runs with, the folder, and any leading options.
pub(crate) struct Git<'a> {
    pub(crate) environment: &'a [(OsString, OsString)],
    pub(crate) at: &'a Path,
    pub(crate) options: Vec<OsString>,
    /// Handed to git by a credential helper that reads it from this one process's
    /// environment, so it never reaches a file, the repository's config, or commands.
    pub(crate) credential: Option<&'a GitCredential>,
}

/// The helper git runs for a credential: it answers `get` from two variables set only on
/// that git process. Any helper configured elsewhere is cleared first (the empty value).
const HELPER: &str = "!f() { test \"$1\" = get || exit 0; \
    printf 'username=%s\\npassword=%s\\n' \"$GLAVE_GIT_USERNAME\" \"$GLAVE_GIT_PASSWORD\"; }; f";

impl<'a> Git<'a> {
    /// git in the folder `at`.
    pub(crate) fn at(environment: &'a [(OsString, OsString)], at: &'a Path) -> Self {
        Self {
            environment,
            at,
            options: Vec::new(),
            credential: None,
        }
    }

    /// Runs git with `args`; its output, or what it said when it failed.
    pub(crate) async fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut command = tokio::process::Command::new("git");
        if self.credential.is_some() {
            command
                .args(["-c", "credential.helper=", "-c"])
                .arg(format!("credential.helper={HELPER}"));
        }
        let output = command
            .args(&self.options)
            .args(args)
            .current_dir(self.at)
            .env_clear()
            .envs(self.environment.iter().map(|(name, value)| (name, value)))
            .envs(self.credential.iter().flat_map(|credential| {
                [
                    ("GLAVE_GIT_USERNAME", credential.username.as_str()),
                    ("GLAVE_GIT_PASSWORD", credential.password.as_str()),
                ]
            }))
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| format!("could not start git: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            let said = String::from_utf8_lossy(&output.stderr);
            Err(format!("git {}: {}", args.join(" "), said.trim()))
        }
    }

    pub(crate) async fn must(&self, args: &[&str]) -> Result<String, ComputerError> {
        self.run(args).await.map_err(ComputerError::Failed)
    }
}

/// Refuses text git would read as an option.
pub(crate) fn not_an_option(what: &str, value: &str) -> Result<(), ComputerError> {
    if value.starts_with('-') || value.trim().is_empty() {
        Err(ComputerError::Refused(format!(
            "{what} {value:?} is not valid"
        )))
    } else {
        Ok(())
    }
}

/// Clones `url` into `dest`, at a branch, tag or commit.
pub(crate) async fn clone(
    environment: &[(OsString, OsString)],
    url: &str,
    at: Option<&str>,
    dest: &Path,
    credential: Option<&GitCredential>,
) -> Result<(), ComputerError> {
    not_an_option("the repository", url)?;
    if let Some(at) = at {
        not_an_option("the revision", at)?;
    }
    let parent = dest
        .parent()
        .ok_or_else(|| ComputerError::Failed(format!("{} has no parent", dest.display())))?;
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        ComputerError::Failed(format!("could not create {}: {error}", parent.display()))
    })?;
    let git = Git {
        environment,
        at: parent,
        options: Vec::new(),
        credential,
    };
    let dest_text = dest.to_string_lossy();
    git.must(&["clone", "--quiet", "--", url, &dest_text])
        .await?;
    if let Some(at) = at {
        let inside = Git {
            environment,
            at: dest,
            options: Vec::new(),
            credential,
        };
        if let Err(error) = inside.run(&["checkout", "--quiet", at, "--"]).await {
            let _ = tokio::fs::remove_dir_all(dest).await;
            return Err(ComputerError::NotFound(error));
        }
    }
    Ok(())
}

/// Commits everything in the workspace to `save.branch`, and pushes it to `origin` if the
/// workspace has one.
pub(crate) async fn save(opened: &Opened, save: &Save) -> Result<Saved, ComputerError> {
    not_an_option("the branch", &save.branch)?;
    if save.message.trim().is_empty() {
        return Err(ComputerError::Refused("the commit message is empty".into()));
    }
    let git = repository(opened).await?;
    if git
        .run(&["check-ref-format", "--branch", &save.branch])
        .await
        .is_err()
    {
        return Err(ComputerError::Refused(format!(
            "{:?} is not a valid branch name",
            save.branch
        )));
    }
    let reference = format!("refs/heads/{}", save.branch);
    let exists = git
        .run(&["rev-parse", "--verify", "--quiet", &reference])
        .await
        .is_ok();
    let born = git
        .run(&["rev-parse", "--verify", "--quiet", "HEAD"])
        .await
        .is_ok();
    if !exists && born {
        git.must(&["branch", "--", &save.branch]).await?;
    }
    // Point HEAD at the branch without touching any file: the commit records the workspace
    // exactly as it is, on top of the branch.
    git.must(&["symbolic-ref", "HEAD", &reference]).await?;
    git.must(&["add", "--all"]).await?;
    let mut commit = Vec::new();
    if git.run(&["config", "user.email"]).await.is_err() {
        commit.extend(["-c", "user.name=glave", "-c", "user.email=glave@localhost"]);
    }
    commit.extend([
        "commit",
        "--quiet",
        "--no-verify",
        "--allow-empty",
        "-m",
        &save.message,
    ]);
    git.must(&commit).await?;
    let hash = git.must(&["rev-parse", "HEAD"]).await?;
    let pushed = if git.run(&["remote", "get-url", "origin"]).await.is_ok() {
        let refspec = format!("{reference}:{reference}");
        git.must(&["push", "--quiet", "origin", &refspec]).await?;
        true
    } else {
        false
    };
    Ok(Saved {
        commit: hash,
        pushed,
    })
}

/// The workspace's repository: its own, or for a plain folder, its hidden history.
async fn repository(opened: &Opened) -> Result<Git<'_>, ComputerError> {
    let mut git = Git {
        environment: &opened.environment,
        at: &opened.root,
        options: Vec::new(),
        credential: opened.credential.as_ref(),
    };
    if tokio::fs::try_exists(opened.root.join(".git"))
        .await
        .unwrap_or(false)
    {
        return Ok(git);
    }
    let size = total_size(&opened.root, MAX_PLAIN_BYTES).await?;
    if size > MAX_PLAIN_BYTES {
        return Err(ComputerError::Refused(format!(
            "this folder holds more than {} MB; glave keeps history for plain folders up to that \
             size. Make it a git repository to save it",
            MAX_PLAIN_BYTES / 1024 / 1024
        )));
    }
    let history = opened.root.join(HISTORY);
    let mut git_dir = OsString::from("--git-dir=");
    git_dir.push(&history);
    let mut work_tree = OsString::from("--work-tree=");
    work_tree.push(&opened.root);
    git.options = vec![git_dir, work_tree];
    if !tokio::fs::try_exists(&history).await.unwrap_or(false) {
        let info = history.join("info");
        let exclude = info.join("exclude");
        let failed = |error: std::io::Error| {
            ComputerError::Failed(format!("could not write {}: {error}", exclude.display()))
        };
        tokio::fs::create_dir_all(&info).await.map_err(failed)?;
        git.must(&["init", "--quiet"]).await?;
        tokio::fs::write(&exclude, format!("/{HISTORY}/\n"))
            .await
            .map_err(failed)?;
        ignore_history(&opened.root).await?;
    }
    Ok(git)
}

/// Makes sure `<folder>/.glave/.gitignore` lists the hidden history, so a `git init` run in
/// the folder later never commits it. Creates the file if it is missing, and otherwise adds
/// the line once, keeping every other line.
pub(crate) async fn ignore_history(root: &Path) -> Result<(), ComputerError> {
    let path = root.join(".glave").join(".gitignore");
    let failed = |error: std::io::Error| {
        ComputerError::Failed(format!("could not write {}: {error}", path.display()))
    };
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(failed(error)),
    };
    let Some(mut text) = text else {
        return tokio::fs::write(&path, format!("{IGNORED}\n"))
            .await
            .map_err(failed);
    };
    if text
        .lines()
        .any(|line| matches!(line.trim(), IGNORED | "/history.git"))
    {
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(IGNORED);
    text.push('\n');
    tokio::fs::write(&path, text).await.map_err(failed)
}
