//! Work moves between computers only through git: a repository is cloned into a workspace,
//! and a save commits everything to a branch and pushes it back. A plain folder lives on one
//! computer and is worked in place; glave saves nothing in it.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use glave_ports::computer::{ComputerError, GitCredential, Save, Saved};

use super::registry::Opened;

/// How long git has for work on this computer (reading a repository, a commit, a checkout or
/// a worktree): moments as a rule, a minute or so in a very large repository.
const LOCAL: Duration = Duration::from_mins(2);

/// How long git has for a clone or a push, which move the repository across the network: as
/// long as glave waits for a computer elsewhere to open or save a workspace.
const NETWORK: Duration = Duration::from_mins(30);

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

    /// Runs git with `args` on this computer, within [`LOCAL`]; its output, or what it said
    /// when it failed.
    pub(crate) async fn run(&self, args: &[&str]) -> Result<String, String> {
        self.run_within(args, LOCAL).await
    }

    /// [`Git::run`], within `limit`: git still running then is stopped, and the error says
    /// how long it had.
    async fn run_within(&self, args: &[&str], limit: Duration) -> Result<String, String> {
        let mut command = glave_process::Program::new("git");
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
                    ("GLAVE_GIT_PASSWORD", credential.password.expose()),
                ]
            }))
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output(limit)
            .await
            .map_err(|error| format!("git {}: {error}", args.first().unwrap_or(&"")))?;
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

    /// [`Git::must`] for work that moves the repository across the network, within
    /// [`NETWORK`].
    async fn must_across(&self, args: &[&str]) -> Result<String, ComputerError> {
        self.run_within(args, NETWORK)
            .await
            .map_err(ComputerError::Failed)
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
    git.must_across(&["clone", "--quiet", "--", url, &dest_text])
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
        git.must_across(&["push", "--quiet", "origin", &refspec])
            .await?;
        true
    } else {
        false
    };
    Ok(Saved {
        commit: hash,
        pushed,
    })
}

/// The workspace's own repository. A folder that is not one is refused: glave keeps no
/// history of its own in it.
async fn repository(opened: &Opened) -> Result<Git<'_>, ComputerError> {
    if !tokio::fs::try_exists(opened.root.join(".git"))
        .await
        .unwrap_or(false)
    {
        return Err(ComputerError::Refused(
            "this folder is not a git repository; glave saves work only in repositories".into(),
        ));
    }
    Ok(Git {
        environment: &opened.environment,
        at: &opened.root,
        options: Vec::new(),
        credential: opened.credential.as_ref(),
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn git_that_never_finishes_is_stopped_at_its_limit_saying_so() {
        let dir = glave_ports::testing::TempDir::new("git-hangs").unwrap();
        let git = dir.join("git");
        std::fs::write(&git, "#!/bin/sh\nexec sleep 300\n").unwrap();
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
        let environment = [(OsString::from("PATH"), dir.as_os_str().to_owned())];
        let ran = glave_ports::block_on!(async {
            tokio::time::pause();
            let local = Git::at(&environment, &dir).run(&["status"]).await;
            let across = Git::at(&environment, &dir).must_across(&["push"]).await;
            (local, across)
        });
        assert_eq!(
            ran.0.unwrap_err(),
            "git status: git did not finish within 2 minutes"
        );
        assert!(
            ran.1
                .unwrap_err()
                .to_string()
                .ends_with("git push: git did not finish within 30 minutes"),
        );
    }
}
