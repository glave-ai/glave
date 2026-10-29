//! Work moves between computers only through git: a repository is cloned into a workspace,
//! and a save commits everything to a branch and pushes it back. A plain folder lives on one
//! computer and is worked in place; glave saves nothing in it.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use glave_ports::computer::{ComputerError, GitCredential, Refusal, Save, Saved};
use glave_process::Processes;

use super::registry::Opened;

/// How long git has to read the repository (a branch, a commit, its status): moments as a
/// rule, a minute or so in a very large one.
const READS_WITHIN: Duration = Duration::from_mins(2);

/// How long git has for work that changes files or crosses the network (a clone, a checkout, a
/// worktree, staging and committing everything, a push), which hooks and large files can make
/// long: as long as glave waits for a computer elsewhere to open or save a workspace.
const WORKS_WITHIN: Duration = Duration::from_mins(30);

/// What git said: its output when it succeeded, what it said when it failed.
pub(crate) type Said = Result<String, String>;

/// One git invocation: the processes it runs as, the environment it runs with, the folder,
/// and any leading options.
pub(crate) struct Git<'a> {
    pub(crate) processes: &'a Processes,
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
    /// git in the folder `at`, run as one of `processes`.
    pub(crate) fn at(
        processes: &'a Processes,
        environment: &'a [(OsString, OsString)],
        at: &'a Path,
    ) -> Self {
        Self {
            processes,
            environment,
            at,
            options: Vec::new(),
            credential: None,
        }
    }

    /// git's answer to `args`, a question about the repository, within [`READS_WITHIN`]; an
    /// error when it gave none (it could not start, or did not finish in time), which a
    /// question of yes or no never takes for a no.
    pub(crate) async fn ask(&self, args: &[&str]) -> Result<Said, ComputerError> {
        self.said(args, READS_WITHIN)
            .await
            .map_err(ComputerError::Failed)
    }

    /// [`Git::ask`], no answer counting as a failure: for a question whose unknown answer
    /// counts against what it would allow (removing a worktree).
    pub(crate) async fn run(&self, args: &[&str]) -> Said {
        self.said(args, READS_WITHIN).await.unwrap_or_else(Err)
    }

    /// A read that must succeed: git's output.
    pub(crate) async fn must(&self, args: &[&str]) -> Result<String, ComputerError> {
        self.ask(args).await?.map_err(ComputerError::Failed)
    }

    /// Work that must succeed, within [`WORKS_WITHIN`]: git's output.
    pub(crate) async fn work(&self, args: &[&str]) -> Result<String, ComputerError> {
        self.said(args, WORKS_WITHIN)
            .await
            .map_err(ComputerError::Failed)?
            .map_err(ComputerError::Failed)
    }

    /// What git said to `args` within `limit`, or why it said nothing: git still running then
    /// is stopped, and the error says how long it had.
    async fn said(&self, args: &[&str], limit: Duration) -> Result<Said, String> {
        let mut command = glave_process::Program::new("git");
        if self.credential.is_some() {
            command
                .args(["-c", "credential.helper=", "-c"])
                .arg(format!("credential.helper={HELPER}"));
        }
        command
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
            .stdin(Stdio::null());
        let output = self
            .processes
            .output(&mut command, limit)
            .await
            .map_err(|error| format!("git {}: {error}", args.first().unwrap_or(&"")))?;
        Ok(if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            let said = String::from_utf8_lossy(&output.stderr);
            Err(format!("git {}: {}", args.join(" "), said.trim()))
        })
    }
}

/// Refuses text git would read as an option.
pub(crate) fn not_an_option(what: fn(String) -> Refusal, value: &str) -> Result<(), ComputerError> {
    if value.starts_with('-') || value.trim().is_empty() {
        Err(ComputerError::Refused(what(format!("{value:?}")).into()))
    } else {
        Ok(())
    }
}

/// Clones `url` into `dest`, at a branch, tag or commit.
pub(crate) async fn clone(
    processes: &Processes,
    environment: &[(OsString, OsString)],
    url: &str,
    at: Option<&str>,
    dest: &Path,
    credential: Option<&GitCredential>,
) -> Result<(), ComputerError> {
    not_an_option(|value| Refusal::RepositoryNotValid { value }, url)?;
    if let Some(at) = at {
        not_an_option(|value| Refusal::RevisionNotValid { value }, at)?;
    }
    let parent = dest
        .parent()
        .ok_or_else(|| ComputerError::Failed(format!("{} has no parent", dest.display())))?;
    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        ComputerError::Failed(format!("could not create {}: {error}", parent.display()))
    })?;
    let git = Git {
        processes,
        environment,
        at: parent,
        options: Vec::new(),
        credential,
    };
    let dest_text = dest.to_string_lossy();
    git.work(&["clone", "--quiet", "--", url, &dest_text])
        .await?;
    if let Some(at) = at {
        let inside = Git {
            processes,
            environment,
            at: dest,
            options: Vec::new(),
            credential,
        };
        let failed = match inside
            .said(&["checkout", "--quiet", at, "--"], WORKS_WITHIN)
            .await
        {
            Ok(Ok(_)) => None,
            Ok(Err(said)) => Some(ComputerError::NotFound(said)),
            Err(unanswered) => Some(ComputerError::Failed(unanswered)),
        };
        if let Some(failed) = failed {
            let _ = tokio::fs::remove_dir_all(dest).await;
            return Err(failed);
        }
    }
    Ok(())
}

/// Commits everything in the workspace to `save.branch`, and pushes it to `origin` if the
/// workspace has one.
pub(crate) async fn save(opened: &Opened, save: &Save) -> Result<Saved, ComputerError> {
    not_an_option(|value| Refusal::BranchNotValid { value }, &save.branch)?;
    if save.message.trim().is_empty() {
        return Err(ComputerError::Refused(Refusal::NoCommitMessage.into()));
    }
    let git = repository(opened).await?;
    if git
        .ask(&["check-ref-format", "--branch", &save.branch])
        .await?
        .is_err()
    {
        return Err(ComputerError::Refused(
            Refusal::NotABranchName {
                branch: format!("{:?}", save.branch),
            }
            .into(),
        ));
    }
    let reference = format!("refs/heads/{}", save.branch);
    let exists = git
        .ask(&["rev-parse", "--verify", "--quiet", &reference])
        .await?
        .is_ok();
    let born = git
        .ask(&["rev-parse", "--verify", "--quiet", "HEAD"])
        .await?
        .is_ok();
    if !exists && born {
        git.must(&["branch", "--", &save.branch]).await?;
    }
    // Point HEAD at the branch without touching any file: the commit records the workspace
    // exactly as it is, on top of the branch.
    git.must(&["symbolic-ref", "HEAD", &reference]).await?;
    git.work(&["add", "--all"]).await?;
    let mut commit = Vec::new();
    if git.ask(&["config", "user.email"]).await?.is_err() {
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
    git.work(&commit).await?;
    let hash = git.must(&["rev-parse", "HEAD"]).await?;
    let pushed = if git.ask(&["remote", "get-url", "origin"]).await?.is_ok() {
        let refspec = format!("{reference}:{reference}");
        git.work(&["push", "--quiet", "origin", &refspec]).await?;
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
        return Err(ComputerError::Refused(Refusal::NotARepository.into()));
    }
    Ok(Git {
        processes: &opened.processes,
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
        // This git first, and the system's own folders after it, where `sleep` is.
        let mut path = dir.as_os_str().to_owned();
        path.push(":/usr/bin:/bin");
        let environment = [(OsString::from("PATH"), path)];
        let processes = Processes::unrecorded();
        let (asked, worked) = glave_ports::block_on!(async {
            tokio::time::pause();
            let asked = Git::at(&processes, &environment, &dir)
                .ask(&["status"])
                .await;
            let worked = Git::at(&processes, &environment, &dir)
                .work(&["push"])
                .await;
            (asked, worked)
        });
        // No answer is never a no: a question's caller gets the error, not git's failure.
        assert_eq!(
            asked.err().map(|error| error.to_string()).as_deref(),
            Some("failed: git status: git did not finish within 2 minutes")
        );
        assert_eq!(
            worked.err().map(|error| error.to_string()).as_deref(),
            Some("failed: git push: git did not finish within 30 minutes")
        );
    }
}
