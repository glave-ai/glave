//! glave's git and what a repository names: each way a repository's config runs a program, shown
//! to run it under plain git and shown not to under glave's, on any platform, since no sandbox
//! is involved. That a sandboxed command can write these files is shown in the sandbox's tests.

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Stdio;

use glave_ports::block_on;
use glave_ports::computer::{Computers, Environment, Sources, WorkspaceInfo};
use glave_ports::testing::TempDir;
use glave_process::Program;

use super::*;
use crate::LocalComputers;
use crate::worktrees::{self, Condition};

/// A repository with a commit, a program it can be made to run, and what that program leaves.
struct Planted {
    dir: TempDir,
    repo: PathBuf,
    program: PathBuf,
    ran: PathBuf,
}

/// Git as a person's terminal runs it, knowing only this test's home.
fn plain(planted: &Planted, at: &Path, args: &[&str]) -> std::process::Output {
    let mut git = Program::new("git");
    git.args(args)
        .current_dir(at)
        .env_clear()
        .envs(environment(&planted.dir))
        .no_input()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    block_on!(Processes::unrecorded().output(&mut git, Duration::from_secs(60))).unwrap()
}

/// `git` with `args` that must succeed; what it printed.
fn must(planted: &Planted, args: &[&str]) -> String {
    let done = plain(planted, &planted.repo, args);
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

/// Only this test's home, with no system settings.
fn environment(dir: &Path) -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([
            ("HOME".into(), dir.join("home").into_os_string()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ])
        .collect()
}

fn planted() -> Planted {
    let dir = TempDir::new("untrusted-git").unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let ran = dir.join("ran");
    let program = dir.join("program");
    executable(&program, &format!("#!/bin/sh\n: > '{}'\n", ran.display()));
    let planted = Planted {
        dir,
        repo,
        program,
        ran,
    };
    must(&planted, &["init", "-q", "--initial-branch=main"]);
    std::fs::write(planted.repo.join("a.txt"), "one\n").unwrap();
    must(&planted, &["add", "a.txt"]);
    must(
        &planted,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "one",
        ],
    );
    std::fs::write(planted.repo.join("a.txt"), "two\n").unwrap();
    must(&planted, &["add", "a.txt"]);
    must(
        &planted,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "two",
        ],
    );
    planted
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl Planted {
    fn config(&self, key: &str, value: &str) {
        must(self, &["config", key, value]);
    }

    /// What ran since it was last asked.
    fn take_ran(&self) -> bool {
        std::fs::remove_file(&self.ran).is_ok()
    }

    /// Every attribute's file says `driver` filters every file.
    fn filter_everything(&self, driver: &str) {
        let info = self.repo.join(".git/info");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("attributes"), format!("* filter={driver}\n")).unwrap();
    }

    /// Plain git, which runs what's planted: the proof that something is.
    fn plain_git_runs_it(&self) {
        let elsewhere = self.dir.join("plain-worktree");
        let elsewhere = elsewhere.to_string_lossy();
        // It need not succeed: a lazy fetch of nothing fails, having run the program.
        let added = plain(
            self,
            &self.repo,
            &["worktree", "add", "-q", "-b", "plain", &elsewhere, "HEAD"],
        );
        std::fs::write(self.repo.join("a.txt"), "changed\n").unwrap();
        let status = plain(self, &self.repo, &["status", "--porcelain"]);
        assert!(
            self.take_ran(),
            "plain git never ran it: {}{}",
            String::from_utf8_lossy(&added.stderr),
            String::from_utf8_lossy(&status.stderr)
        );
        std::fs::write(self.repo.join("a.txt"), "two\n").unwrap();
    }

    /// Glave makes a worktree of the repository.
    fn glave_opens_a_worktree(&self) -> Result<WorkspaceInfo, ComputerError> {
        let computers = LocalComputers::new(self.dir.join("copies"), environment(&self.dir))
            .with_worktrees(self.dir.join("worktrees"));
        let sources = Sources::Worktree {
            path: self.repo.to_string_lossy().into_owned(),
            branch: "glave/thread".into(),
            owner: None,
        };
        block_on!(async {
            let computer = computers.create(&Environment::host()).await.unwrap();
            computers.open(computer.id, &sources).await
        })
    }

    /// Glave makes a worktree of the repository, a file changes in it, and glave looks at the
    /// worktrees it keeps, which runs `status`: nothing of the repository's has run, and git
    /// still tells what changed.
    fn glave_s_git_runs_nothing(&self) {
        let opened = self.glave_opens_a_worktree().unwrap();
        std::fs::write(Path::new(&opened.root).join("a.txt"), "changed\n").unwrap();
        let kept = block_on!(worktrees::list(
            &self.dir.join("worktrees"),
            &environment(&self.dir),
            &Processes::unrecorded()
        ))
        .unwrap();
        let [kept] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        assert_eq!(
            kept.condition,
            Condition::Unsafe {
                changed: true,
                unpushed: 2
            }
        );
        assert!(
            !self.ran.exists(),
            "glave's git ran what the repository named"
        );
    }
}

#[test]
fn a_hook_in_the_repository_s_own_folder_never_runs() {
    let planted = planted();
    executable(
        &planted.repo.join(".git/hooks/post-checkout"),
        &format!("#!/bin/sh\n'{}'\n", planted.program.display()),
    );
    planted.plain_git_runs_it();
    planted.glave_s_git_runs_nothing();
}

#[test]
fn a_folder_of_hooks_the_config_names_never_runs() {
    let planted = planted();
    let hooks = planted.dir.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    executable(
        &hooks.join("post-checkout"),
        &format!("#!/bin/sh\n'{}'\n", planted.program.display()),
    );
    planted.config("core.hooksPath", &hooks.to_string_lossy());
    planted.plain_git_runs_it();
    planted.glave_s_git_runs_nothing();
}

#[test]
fn a_file_system_monitor_the_config_names_never_runs() {
    let planted = planted();
    planted.config("core.fsmonitor", &planted.program.to_string_lossy());
    planted.plain_git_runs_it();
    planted.glave_s_git_runs_nothing();
}

#[test]
fn a_filter_the_config_names_never_runs() {
    for driver in ["planted", "named=with-an-equals-sign"] {
        let planted = planted();
        planted.filter_everything(driver);
        for part in ["clean", "smudge"] {
            planted.config(
                &format!("filter.{driver}.{part}"),
                &planted.program.to_string_lossy(),
            );
        }
        planted.plain_git_runs_it();
        planted.glave_s_git_runs_nothing();
    }
}

#[test]
fn a_filter_the_person_set_for_themselves_keeps_running() {
    let planted = planted();
    planted.filter_everything("theirs");
    std::fs::write(
        planted.dir.join("home/.gitconfig"),
        format!(
            "[filter \"theirs\"]\n\tsmudge = {}\n",
            planted.program.display()
        ),
    )
    .unwrap();
    planted.glave_opens_a_worktree().unwrap();
    assert!(planted.ran.exists(), "a person's own filter was skipped");
}

/// The config makes a remote of its own the place missing objects come from, and a remote helper
/// its address.
fn plant_a_lazy_fetch(planted: &Planted) {
    planted.config("core.repositoryformatversion", "1");
    planted.config("extensions.partialClone", "planted");
    planted.config("remote.planted.promisor", "true");
    let url = format!("ext::sh -c {}", planted.program.display());
    planted.config("remote.planted.url", &url);
    planted.config("protocol.ext.allow", "always");
}

/// Removes the object `name`, so reading it needs a fetch.
fn lose(planted: &Planted, name: &str) {
    let hash = must(planted, &["rev-parse", name]);
    let (folder, file) = hash.split_at(2);
    std::fs::remove_file(planted.repo.join(".git/objects").join(folder).join(file)).unwrap();
}

#[test]
fn a_fetch_of_a_missing_file_never_runs_what_the_config_names() {
    let planted = planted();
    lose(&planted, "HEAD:a.txt");
    plant_a_lazy_fetch(&planted);
    planted.plain_git_runs_it();
    // No transport, so no worktree where a file is missing: what a real partial clone costs too.
    let refused = planted
        .glave_opens_a_worktree()
        .err()
        .map(|error| error.to_string());
    assert!(
        refused
            .as_deref()
            .is_some_and(|said| said.contains("not allowed")),
        "{refused:?}"
    );
    assert!(
        !planted.ran.exists(),
        "glave's git ran what the repository named"
    );
}

#[test]
fn a_question_about_a_repository_never_fetches_what_the_config_names() {
    let planted = planted();
    lose(&planted, "HEAD");
    plant_a_lazy_fetch(&planted);
    let asked = ["rev-list", "HEAD", "--not", "--remotes"];
    assert!(!plain(&planted, &planted.repo, &asked).status.success());
    assert!(planted.take_ran(), "plain git never ran it");
    let said = block_on!(ask_repository(
        &Processes::unrecorded(),
        &planted.repo,
        &asked
    ));
    assert!(said.is_err(), "{said:?}");
    assert!(
        !planted.ran.exists(),
        "the question ran what the repository named"
    );
}

#[test]
fn a_repository_s_settings_are_named_by_scope_and_only_those_that_name_a_program() {
    let listed = b"global\0filter.lfs.smudge\0local\0filter.a=b c.clean\0local\0core.bare\0\
        worktree\0lfs.customtransfer.x.path\0local\0lfs.url\0command\0filter.c.clean\0";
    assert_eq!(
        named_by_repository(listed).unwrap(),
        ["filter.a=b c.clean", "lfs.customtransfer.x.path"]
    );
    assert!(
        named_by_repository(b"local\0filter.\xff.clean\0").is_none(),
        "one that can't be said back to git is no reason to run it"
    );
    assert_eq!(
        named_by_repository(b"local\0branch.\xff.remote\0").unwrap(),
        Vec::<String>::new(),
        "any other name may be anything"
    );
}

#[test]
fn git_that_never_finishes_is_stopped_at_its_limit_saying_so() {
    let dir = TempDir::new("git-hangs").unwrap();
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
