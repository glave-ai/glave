//! Work moves between computers only through git: a repository is cloned into a workspace,
//! and a save commits everything to a branch and pushes it back. A plain folder lives on one
//! computer and is worked in place; glave saves nothing in it.
//!
//! On the person's machine it runs outside the sandbox, and a sandboxed command can write the
//! config and hooks of any repository in a folder it may write: only a workspace's own root is
//! protected, and a repository can be made, or swapped in, at any time. So what is in a repository
//! is not trusted to name a program (`specs/sandbox.md`, "Repositories inside the project"). A
//! clone and a save happen only on a worker, inside its own wall, where the config and the home
//! folder are its commands' to write: only the git that clones and the git that pushes may cross
//! the network or hold a credential, and a push holding one reads no config they wrote.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use glave_ports::computer::{ComputerError, GitCredential, Refusal, Save, Saved};
use glave_process::Processes;

use super::registry::{Opened, Pushes, new_id};

/// How long git has to read the repository (a branch, a commit, its status): moments as a
/// rule, a minute or so in a very large one.
const READS_WITHIN: Duration = Duration::from_mins(2);

/// How long git has for work that changes files or crosses the network (a clone, a checkout, a
/// worktree, staging and committing everything, a push), which large files can make long: as
/// long as glave waits for a computer elsewhere to open or save a workspace.
const WORKS_WITHIN: Duration = Duration::from_mins(30);

/// What git said: its output when it succeeded, what it said when it failed.
pub(crate) type Said = Result<String, String>;

/// What every git of glave's runs with: no hooks, the repository's or the person's, and no
/// file-system monitor, which a repository's config can name a program for.
const RUNS_NOTHING: [(&str, &str); 2] = [("core.hooksPath", "/dev/null"), ("core.fsmonitor", "")];

/// What a repository's own config may name a program under, in ways no fixed setting reaches:
/// filter drivers, by a name it chooses, and what git-lfs runs when the person's own filter
/// fetches a file. Blanked where the repository set them, so the person's own keep running.
const NAMES_A_PROGRAM: [&str; 6] = [
    "filter.",
    "lfs.customtransfer.",
    "lfs.extension.",
    "core.sshcommand",
    "core.askpass",
    "credential.",
];

/// The commands of git's that read no file through a filter, so need no reading of the config
/// first: every other does, and starting a program is the slow part of asking git.
const RUNS_NO_FILTER: [&str; 13] = [
    "branch",
    "check-ignore",
    "check-ref-format",
    "clone",
    "config",
    "init",
    "ls-files",
    "push",
    "remote",
    "rev-list",
    "rev-parse",
    "symbolic-ref",
    "update-ref",
];

/// The transports git may use when a command should cross no network, none being called this:
/// a repository's config can make any read fetch a missing object through a remote it names,
/// and with that run a helper, an ssh command or a remote helper.
const NO_TRANSPORT: &str = "none";

/// What keeps a git from the config files outside its repository, which are a worker's commands'
/// to write: the machine's and the home folder's.
const READS_NO_CONFIG: [(&str, &str); 2] = [
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
];

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
    /// Whether this git may cross the network: only to clone or push.
    pub(crate) network: bool,
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
            network: false,
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
        let mut command = match self.prepared(args).await? {
            Ok(command) => command,
            Err(said) => return Ok(Err(said)),
        };
        let output = self
            .processes
            .output(&mut command, limit)
            .await
            .map_err(|error| unanswered(args, &error))?;
        Ok(if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(failure(args, &output))
        })
    }

    /// What git printed for `args`, a read, within [`READS_WITHIN`]: the first `most` bytes of
    /// it, and whether it printed more, when git was stopped there; what it said when it failed.
    pub(crate) async fn printed(
        &self,
        args: &[&str],
        most: usize,
    ) -> Result<Result<(Vec<u8>, bool), String>, ComputerError> {
        let mut command = match self.prepared(args).await.map_err(ComputerError::Failed)? {
            Ok(command) => command,
            Err(said) => return Ok(Err(said)),
        };
        let output = self
            .processes
            .output_at_most(&mut command, READS_WITHIN, most);
        let mut output = output
            .await
            .map_err(|error| ComputerError::Failed(unanswered(args, &error)))?;
        let more = output.stdout.len() > most;
        output.stdout.truncate(most);
        Ok(if output.status.success() || more {
            Ok((output.stdout, more))
        } else {
            Err(failure(args, &output))
        })
    }

    /// git with `args`, ready to run with every setting a repository named a program under
    /// blanked; what git said when its settings could not be read; or why it said nothing.
    async fn prepared(
        &self,
        args: &[&str],
    ) -> Result<Result<glave_process::Program, String>, String> {
        let blanked = if args
            .first()
            .is_some_and(|command| RUNS_NO_FILTER.contains(command))
        {
            Vec::new()
        } else {
            let mut listing =
                self.program(&["config", "--show-scope", "--null", "--name-only", "--list"]);
            let listed = self
                .processes
                .output(&mut listing, READS_WITHIN)
                .await
                .map_err(|error| unanswered(args, &error))?;
            if !listed.status.success() {
                return Ok(Err(failure(args, &listed)));
            }
            let Some(named) = named_by_repository(&listed.stdout) else {
                return Ok(Err(format!(
                    "git {}: this repository's settings have a name git can't be told to skip",
                    args.join(" ")
                )));
            };
            named
        };
        let mut command = self.program(args);
        command.envs(config_settings(
            self.environment,
            RUNS_NOTHING
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .chain(blanked.iter().flat_map(|key| blanking(key))),
        ));
        Ok(Ok(command))
    }

    /// git with `args`, in this folder and environment.
    fn program(&self, args: &[&str]) -> glave_process::Program {
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
            .no_input();
        if !self.network {
            command.env("GIT_ALLOW_PROTOCOL", NO_TRANSPORT);
        }
        command
    }
}

/// The settings that blank `key`. A filter blanked is also required, so a file it would have
/// changed fails loudly rather than pass unencrypted or unconverted, as with git-crypt's.
fn blanking(key: &str) -> Vec<(String, String)> {
    let mut settings = vec![(key.to_owned(), String::new())];
    if let (true, Some((driver, _))) = (key.starts_with("filter."), key.rsplit_once('.')) {
        settings.push((format!("{driver}.required"), "true".to_owned()));
    }
    settings
}

/// Why git gave no answer to `args`: `error`.
fn unanswered(args: &[&str], error: &glave_process::ProcessError) -> String {
    format!("git {}: {error}", args.first().unwrap_or(&""))
}

/// What git said when it failed at `args`.
fn failure(args: &[&str], output: &std::process::Output) -> String {
    let said = String::from_utf8_lossy(&output.stderr);
    format!("git {}: {}", args.join(" "), said.trim())
}

/// In `git config --show-scope --null --name-only --list`'s output, the keys the repository's own
/// config sets under [`NAMES_A_PROGRAM`]; `None` for a key that isn't text, which could not be
/// named back to git to blank it.
fn named_by_repository(listed: &[u8]) -> Option<Vec<String>> {
    let mut parts = listed.split(|byte| *byte == 0);
    let mut named = Vec::new();
    while let (Some(scope), Some(key)) = (parts.next(), parts.next()) {
        if !matches!(scope, b"local" | b"worktree") {
            continue;
        }
        if NAMES_A_PROGRAM
            .iter()
            .any(|name| key.starts_with(name.as_bytes()))
        {
            named.push(String::from_utf8(key.to_vec()).ok()?);
        }
    }
    Some(named)
}

/// `settings` for git, through `GIT_CONFIG_*` after any `environment` has already: they outrank
/// every config file, and unlike `git -c` take any key, whatever a repository named it.
pub(crate) fn config_settings(
    environment: &[(OsString, OsString)],
    settings: impl Iterator<Item = (String, String)>,
) -> Vec<(OsString, OsString)> {
    let mut n = environment
        .iter()
        .find(|(name, _)| name == "GIT_CONFIG_COUNT")
        .and_then(|(_, count)| count.to_str()?.parse::<usize>().ok())
        .unwrap_or(0);
    let mut variables = Vec::new();
    for (key, value) in settings {
        variables.push((format!("GIT_CONFIG_KEY_{n}").into(), key.into()));
        variables.push((format!("GIT_CONFIG_VALUE_{n}").into(), value.into()));
        n = n.saturating_add(1);
    }
    variables.push(("GIT_CONFIG_COUNT".into(), n.to_string().into()));
    variables
}

/// git's answer to `args`, a question about the repository around `folder`, asked as the server
/// itself: for a program that reads a repository before any computer holds it.
pub async fn ask_repository(
    processes: &Processes,
    folder: &Path,
    args: &[&str],
) -> Result<String, ComputerError> {
    let environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    Git::at(processes, &environment, folder).must(args).await
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
        network: true,
    };
    let dest_text = dest.to_string_lossy();
    git.work(&["clone", "--quiet", "--", url, &dest_text])
        .await?;
    if let Some(at) = at {
        let inside = Git::at(processes, environment, dest);
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

/// Commits everything in the workspace to `save.branch`, and pushes it: a clone opened with a
/// credential to where it was cloned from, whatever its config says now; any other repository to
/// its `origin`, if it has one, there being no credential to protect.
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
        "--no-gpg-sign",
        "--allow-empty",
        "-m",
        &save.message,
    ]);
    git.work(&commit).await?;
    let hash = git.must(&["rev-parse", "HEAD"]).await?;
    let pushed = if let Some(pushes) = &opened.pushes {
        push_with_credential(opened, &git, pushes, &format!("{hash}:{reference}")).await?;
        git.must(&["update-ref", &tracking(&save.branch), &hash])
            .await?;
        true
    } else if git.ask(&["remote", "get-url", "origin"]).await?.is_ok() {
        let pushing = Git {
            network: true,
            ..Git::at(&opened.processes, &opened.environment, &opened.root)
        };
        let refspec = format!("{reference}:{reference}");
        pushing
            .work(&["push", "--quiet", "origin", &refspec])
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

/// Where this repository remembers `branch` on its remote, which a plain push would have updated:
/// what tells a release there is nothing left to save.
fn tracking(branch: &str) -> String {
    format!("refs/remotes/origin/{branch}")
}

/// Pushes `refspec` to where `pushes` says, from a repository made for it that borrows this one's
/// objects and nothing else: the git holding the credential reads no config the workspace's
/// commands could write. Blanking settings, as glave's other git does, wouldn't hold here: a blanked
/// `http.sslVerify` is false, and a planted `http.<url>.proxy` outranks a blanked `http.proxy`. The
/// URL is given as a setting, not an argument, so an error never prints what it may hold.
async fn push_with_credential(
    opened: &Opened,
    git: &Git<'_>,
    pushes: &Pushes,
    refspec: &str,
) -> Result<(), ComputerError> {
    let found = git
        .must(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
            "--git-path",
            "shallow",
            "--show-object-format",
        ])
        .await?;
    let mut found = found.lines();
    let (Some(objects), Some(shallow), Some(format)) = (found.next(), found.next(), found.next())
    else {
        return Err(ComputerError::Failed(format!(
            "git rev-parse told too little about {}",
            opened.root.display()
        )));
    };
    let objects = Path::new(objects);
    let own = objects
        .with_file_name(format!("glave-push-{}", new_id()?))
        .into_os_string();
    let environment: Vec<(OsString, OsString)> = opened
        .environment
        .iter()
        .cloned()
        .chain(READS_NO_CONFIG.map(|(name, value)| (name.into(), value.into())))
        .chain([("GIT_SHALLOW_FILE".into(), shallow.into())])
        .collect();
    let outside = Git::at(&opened.processes, &environment, &opened.root);
    let own_text = own.to_string_lossy();
    let tried = async {
        // No template: one is a folder of files, a `config` among them, copied in.
        outside
            .must(&[
                "init",
                "--quiet",
                "--bare",
                "--template=",
                &format!("--object-format={format}"),
                &own_text,
            ])
            .await?;
        let borrowed = Path::new(&own).join("objects/info/alternates");
        tokio::fs::write(&borrowed, format!("{}\n", objects.display()))
            .await
            .map_err(|error| {
                ComputerError::Failed(format!("could not write {}: {error}", borrowed.display()))
            })?;
        let pushing = Git {
            options: vec![
                "--git-dir".into(),
                own.clone(),
                "-c".into(),
                format!("remote.origin.url={}", pushes.url).into(),
            ],
            credential: Some(&pushes.credential),
            network: true,
            ..outside
        };
        pushing.work(&["push", "--quiet", "origin", refspec]).await
    };
    let tried = tried.await;
    let _ = tokio::fs::remove_dir_all(&own).await;
    tried.map(|_| ())
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
    Ok(Git::at(
        &opened.processes,
        &opened.environment,
        &opened.root,
    ))
}

#[cfg(test)]
mod tests;
