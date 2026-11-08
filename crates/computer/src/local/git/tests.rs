//! glave's git and what a repository names: each way a repository's config runs a program, shown
//! to run it under plain git and shown not to under glave's, on any platform, since no sandbox
//! is involved. That a sandboxed command can write these files is shown in the sandbox's tests.

use std::path::PathBuf;
use std::process::Stdio;

use glave_ports::block_on;
use glave_ports::computer::{Computers, Environment, Sources, WorkspaceInfo};
use glave_ports::testing::{TempDir, new_script};
use glave_process::Program;

use super::*;
use crate::LocalComputers;
use crate::worktrees::{self, Condition};

mod changes;
mod saving;

/// `a.txt` changed but its size not, so git must read it, through any filter, to see the change.
const SAME_SIZE: &str = "TWO\n";

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
    new_script(&program, format!("#!/bin/sh\n: > '{}'\n", ran.display())).unwrap();
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

impl Planted {
    fn config(&self, key: &str, value: &str) {
        must(self, &["config", key, value]);
    }

    /// A repository at `inner`, with a commit, whose own config names the program as a filter
    /// for every file, and a file there changed, so git looking inside runs it.
    fn repository_inside(&self, inner: &Path) {
        let identity = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
        std::fs::create_dir_all(inner).unwrap();
        assert!(plain(self, inner, &["init", "-q"]).status.success());
        std::fs::write(inner.join("f.txt"), "one\n").unwrap();
        assert!(plain(self, inner, &["add", "f.txt"]).status.success());
        let commit = [&identity[..], &["commit", "-qm", "one"]].concat();
        assert!(plain(self, inner, &commit).status.success());
        let key = "filter.planted.clean";
        let program = self.program.to_string_lossy();
        assert!(
            plain(self, inner, &["config", key, &program])
                .status
                .success()
        );
        std::fs::write(inner.join(".git/info/attributes"), "* filter=planted\n").unwrap();
        std::fs::write(inner.join("f.txt"), "ONE\n").unwrap();
    }

    /// Commits `path` in the repository at `root`: a repository inside, as one it tracks.
    fn holds(&self, root: &Path, path: &str) {
        let identity = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
        assert!(plain(self, root, &["add", path]).status.success());
        let commit = [&identity[..], &["commit", "-qm", "holds it"]].concat();
        assert!(plain(self, root, &commit).status.success());
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
        std::fs::write(self.repo.join("a.txt"), SAME_SIZE).unwrap();
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

    /// The one worktree glave keeps, as it looks at it.
    fn kept(&self) -> worktrees::Kept {
        let kept = block_on!(worktrees::list(
            &self.dir.join("worktrees"),
            &environment(&self.dir),
            &Processes::unrecorded()
        ))
        .unwrap();
        let [kept] = kept.as_slice() else {
            panic!("{kept:?}");
        };
        kept.clone()
    }

    /// Glave makes a worktree of the repository, a file changes in it, and glave looks at the
    /// worktrees it keeps, which runs `status`: nothing of the repository's has run, and git
    /// still tells what changed.
    fn glave_s_git_runs_nothing(&self) {
        let opened = self.glave_opens_a_worktree().unwrap();
        std::fs::write(Path::new(&opened.root).join("a.txt"), SAME_SIZE).unwrap();
        let kept = self.kept();
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
    new_script(
        &planted.repo.join(".git/hooks/post-checkout"),
        format!("#!/bin/sh\n'{}'\n", planted.program.display()),
    )
    .unwrap();
    planted.plain_git_runs_it();
    planted.glave_s_git_runs_nothing();
}

#[test]
fn a_folder_of_hooks_the_config_names_never_runs() {
    let planted = planted();
    let hooks = planted.dir.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    new_script(
        &hooks.join("post-checkout"),
        format!("#!/bin/sh\n'{}'\n", planted.program.display()),
    )
    .unwrap();
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

/// The names a repository may give a filter: `=` is why settings don't go through `git -c`.
const DRIVERS: [&str; 2] = ["planted", "named=with-an-equals-sign"];

#[test]
fn a_filter_the_config_names_refuses_a_worktree_rather_than_run() {
    for driver in DRIVERS {
        let planted = planted();
        planted.filter_everything(driver);
        planted.config(
            &format!("filter.{driver}.smudge"),
            &planted.program.to_string_lossy(),
        );
        planted.plain_git_runs_it();
        let refused = planted
            .glave_opens_a_worktree()
            .err()
            .map(|error| error.to_string());
        assert!(
            refused
                .as_deref()
                .is_some_and(|said| said.contains("filter")),
            "{refused:?}"
        );
        assert!(!planted.ran.exists(), "glave's git ran {driver}");
    }
}

#[test]
fn a_filter_the_config_names_never_runs_when_glave_looks_at_a_worktree() {
    for driver in DRIVERS {
        let planted = planted();
        let opened = planted.glave_opens_a_worktree().unwrap();
        planted.filter_everything(driver);
        planted.config(
            &format!("filter.{driver}.clean"),
            &planted.program.to_string_lossy(),
        );
        planted.plain_git_runs_it();
        std::fs::write(Path::new(&opened.root).join("a.txt"), SAME_SIZE).unwrap();
        // It can't say, which counts against removing it.
        let kept = planted.kept();
        assert!(
            matches!(kept.condition, Condition::Unsafe { changed: true, .. }),
            "{kept:?}"
        );
        assert!(!planted.ran.exists(), "glave's git ran {driver}");
    }
}

#[test]
fn a_filter_in_a_repository_inside_a_worktree_never_runs_and_keeps_it_from_removal() {
    let planted = planted();
    let opened = planted.glave_opens_a_worktree().unwrap();
    let root = PathBuf::from(&opened.root);
    planted.repository_inside(&root.join("inner"));
    planted.holds(&root, "inner");
    plain(&planted, &root, &["status", "--porcelain"]);
    assert!(planted.take_ran(), "plain git never ran it");
    let kept = planted.kept();
    assert!(
        matches!(kept.condition, Condition::Unsafe { changed: true, .. }),
        "{kept:?}"
    );
    let rebuildable = block_on!(worktrees::only_rebuildable(
        &kept,
        &environment(&planted.dir),
        &Processes::unrecorded()
    ));
    assert!(!rebuildable.unwrap());
    assert!(
        !planted.ran.exists(),
        "glave's git ran the inner repository's filter"
    );
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
fn a_filter_blanked_is_required_and_nothing_else_is() {
    let filter = blanking("filter.a=b.clean");
    assert_eq!(
        filter,
        [
            ("filter.a=b.clean".to_owned(), String::new()),
            ("filter.a=b.required".to_owned(), "true".to_owned())
        ]
    );
    assert_eq!(blanking("core.sshcommand").len(), 1);
}

#[test]
fn a_repository_s_settings_are_named_by_scope_and_only_those_that_name_a_program() {
    let listed = b"global\0filter.lfs.smudge\0local\0filter.a=b c.clean\0local\0core.bare\0\
        worktree\0lfs.customtransfer.x.path\0local\0core.sshcommand\0local\0lfs.url\0command\0filter.c.clean\0";
    assert_eq!(
        named_by_repository(listed).unwrap(),
        [
            "filter.a=b c.clean",
            "lfs.customtransfer.x.path",
            "core.sshcommand"
        ]
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
    new_script(&git, "#!/bin/sh\nexec sleep 300\n").unwrap();
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
