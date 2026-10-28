//! A sandboxed worktree, under the real sandbox: it writes its own folder and its repository's
//! shared git folder, so `git commit` works, and nothing else of the repository's. Runs only
//! where the sandbox is (macOS, Linux with bubblewrap), unless `GLAVE_REQUIRE_SANDBOX` says
//! it must.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use glave_ports::block_on;
use glave_ports::computer::{
    Command, ComputerError, Computers, Ending, Environment, Output, Sandbox, SandboxLevel, Sources,
    WorkspaceId,
};

use super::{CACHE_FOLDER, sandbox_available};
use crate::{CancelSwitch, LocalComputers};

fn enforced() -> bool {
    let available = sandbox_available();
    assert!(
        available || std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
        "GLAVE_REQUIRE_SANDBOX is set but the sandbox is not available here"
    );
    available
}

/// A folder outside the system temp folder, which the sandbox lets every command write.
fn scratch(name: &str) -> glave_ports::testing::TempDir {
    let dir = glave_ports::testing::TempDir::outside_temp(name).unwrap();
    std::fs::create_dir_all(dir.join("app")).unwrap();
    dir
}

#[derive(Default)]
struct Collect(std::sync::Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> glave_ports::BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}

async fn run(
    computers: &LocalComputers,
    workspace: WorkspaceId,
    sandbox: &Sandbox,
    script: &str,
) -> (Ending, String) {
    let mut command = Command::new(script);
    command.sandbox = Some(sandbox.clone());
    let output = Collect::default();
    let finished = computers
        .run(workspace, &command, &output, &CancelSwitch::default())
        .await
        .unwrap();
    (finished.ending, output.0.lock().unwrap().clone())
}

/// `dir/app`, opened in place on a new computer.
async fn open_app(computers: &LocalComputers, dir: &Path) -> WorkspaceId {
    let computer = computers.create(&Environment::host()).await.unwrap();
    let app = Sources::Folder {
        path: dir.join("app").to_string_lossy().into_owned(),
    };
    computers.open(computer.id, &app).await.unwrap().id
}

/// A repository with one commit on `main`, in `dir/<name>`.
fn committed_repo(dir: &Path, name: &str) -> PathBuf {
    let repo = dir.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    let made = Process::new("bash")
        .args([
            "-c",
            "git init -q --initial-branch=main && echo hi > a.txt && git add a.txt && \
             git -c user.name=t -c user.email=t@example.com commit -q -m one",
        ])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(made.success());
    std::fs::canonicalize(repo).unwrap()
}

/// This machine's computers, keeping worktrees in glave's folder under `dir/home`, and the
/// sandbox protecting that home.
fn computers(dir: &Path) -> (LocalComputers, Sandbox) {
    let home = dir.join("home");
    let glave_home = home.join(".glave");
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), home.clone().into_os_string())])
        .collect();
    let computers = LocalComputers::new(dir.join("copies"), environment)
        .with_worktrees(glave_home.join("worktrees"));
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: false,
        home: Some(home),
        glave_home: Some(glave_home),
    };
    (computers, sandbox)
}

/// What a sandboxed command must still be able to do with git.
const GIT_WORKS: &str = "export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com \
    GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com && \
    echo two > b.txt && git add b.txt && git commit -q -m two && \
    git branch side && git checkout -q side && git checkout -q - && \
    echo three >> a.txt && git stash -q && git stash list | grep -q 'stash@' && \
    git stash pop -q && git commit -qam three && echo git-works";

async fn git_works(computers: &LocalComputers, workspace: WorkspaceId, sandbox: &Sandbox) {
    let (ending, said) = run(computers, workspace, sandbox, GIT_WORKS).await;
    assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
    assert!(said.contains("git-works"), "{said}");
}

/// `plant <folder>`: a folder holding a hook and a git config, ready to be moved into place.
const PLANT: &str = "plant() { mkdir -p \"$1/hooks\" && echo x > \"$1/hooks/pre-commit\" && \
    echo x > \"$1/config\"; }; ";

/// Where a sandboxed command prepares folders to move in whole: outside the project but on
/// its volume, in the sandbox's cache folder, which sits in glave's folder beside the project.
/// Never the command's temp folder: where the system's is on another volume, the kernel
/// refuses a rename from it before Seatbelt is asked, and `mv` copies, then deletes, so no
/// rename is tested, and the delete empties what the command may write in the git folders
/// (#687).
fn aside_folder(sandbox: &Sandbox) -> PathBuf {
    let glave_home = sandbox.glave_home.as_ref().unwrap();
    let aside = glave_home.join(CACHE_FOLDER).join("aside");
    std::fs::create_dir_all(&aside).unwrap();
    aside
}

/// Fails unless each of `prepared` is still in `aside`: so the command could write there, and
/// nothing moved what it prepared.
fn still_aside(aside: &Path, prepared: &[&str], said: &str) {
    let gone: Vec<&&str> = prepared
        .iter()
        .filter(|prepared| !aside.join(prepared).exists())
        .collect();
    assert!(gone.is_empty(), "no longer aside: {gone:?}: {said}");
}

/// What the computer is told to hide is unreadable to its sandboxed commands, through a link
/// too, while what sits beside it stays readable.
#[test]
fn what_the_computer_hides_is_unreadable() {
    if !enforced() {
        return;
    }
    let dir = scratch("hidden");
    let cargo = dir.join("cargo");
    std::fs::create_dir_all(cargo.join("registry")).unwrap();
    std::fs::create_dir_all(cargo.join("bin")).unwrap();
    std::fs::write(cargo.join("registry/lib.rs"), "canary-hidden\n").unwrap();
    std::fs::write(cargo.join("bin/tool"), "beside-ok\n").unwrap();
    std::os::unix::fs::symlink(cargo.join("registry"), dir.join("app/linked")).unwrap();
    let (computers, sandbox) = computers(&dir);
    let computers = computers.with_hidden([cargo.join("registry")]);
    block_on!(async {
        let here = open_app(&computers, &dir).await;
        let reads = format!(
            "cat '{0}/registry/lib.rs' linked/lib.rs '{0}/bin/tool'; true",
            cargo.display()
        );
        let (_, said) = run(&computers, here, &sandbox, &reads).await;
        assert!(said.contains("beside-ok"), "{said}");
        assert!(!said.contains("canary-hidden"), "{said}");
    });
}

/// A script at `path` that prints `said` and its arguments, standing in for a glave.
pub(super) fn fake_glave(path: &Path, said: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\necho {said} \"$@\"\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A command's `glave` is the glave that runs it, through glave's own folder first on its
/// `PATH`, which the sandbox keeps readable though it sits in glave's hidden folder: with no
/// glave on the `PATH` it was given, and with an older one earlier on it.
#[test]
fn the_running_glave_is_first_on_every_command_s_path() {
    if !enforced() {
        return;
    }
    let dir = scratch("on-path");
    let home = dir.join("home");
    let glave_home = home.join(".glave");
    let on_path = glave_home.join("bin");
    let running = dir.join("running/glave");
    fake_glave(&running, "running-glave");
    fake_glave(&dir.join("old/glave"), "old-glave");
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: false,
        home: Some(home.clone()),
        glave_home: Some(glave_home.clone()),
    };
    let old_first = format!("{}:/usr/bin:/bin", dir.join("old").display());
    block_on!(async {
        crate::glave_on_path(&on_path, &running).await.unwrap();
        for path in ["/usr/bin:/bin", old_first.as_str()] {
            let environment: Vec<(OsString, OsString)> = vec![
                ("PATH".into(), path.into()),
                ("HOME".into(), home.clone().into_os_string()),
            ];
            let computers =
                LocalComputers::new(dir.join("copies"), environment).with_glave_on_path(&on_path);
            let here = open_app(&computers, &dir).await;
            let (ending, said) = run(&computers, here, &sandbox, "glave --version").await;
            assert_eq!(ending, Ending::Exited { code: 0 }, "{path}: {said}");
            assert!(said.contains("running-glave --version"), "{path}: {said}");
            assert!(!said.contains("old-glave"), "{path}: {said}");
            // Outside the sandbox too.
            let unsandboxed = Command::new("glave --version");
            let output = Collect::default();
            let finished = computers
                .run(here, &unsandboxed, &output, &CancelSwitch::default())
                .await
                .unwrap();
            let said = output.0.lock().unwrap().clone();
            assert_eq!(finished.ending, Ending::Exited { code: 0 }, "{said}");
            assert!(said.contains("running-glave"), "{path}: {said}");
        }
    });
}

/// glave's folder stays hidden from a command for its whole life (`specs/invariants.md`, 23):
/// whether the workspace is outside it or a thread's worktree inside it.
#[test]
fn what_glave_writes_in_its_folder_while_a_command_runs_stays_hidden() {
    if !enforced() {
        return;
    }
    let dir = scratch("glave-home");
    let repo = committed_repo(&dir, "repo");
    let (computers, sandbox) = computers(&dir);
    std::fs::create_dir_all(sandbox.glave_home.as_ref().unwrap()).unwrap();
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let app = Sources::Folder {
            path: dir.join("app").to_string_lossy().into_owned(),
        };
        let worktree = Sources::Worktree {
            path: repo.to_string_lossy().into_owned(),
            branch: "glave/late".into(),
            owner: None,
        };
        for sources in [app, worktree] {
            let opened = computers.open(computer.id, &sources).await.unwrap();
            let root = PathBuf::from(&opened.root);
            written_later_stays_hidden(&computers, opened.id, &root, &sandbox).await;
        }
    });
}

/// Runs a command in `workspace` whose root is `root`, under `sandbox`, that reads glave's
/// folder only once glave has replaced a file there (as it replaces its own, by renaming a new
/// one over it) and written a new one, as a sign-in link's page is; it must read neither, and
/// its package cache there must still work.
pub(super) async fn written_later_stays_hidden(
    computers: &LocalComputers,
    workspace: WorkspaceId,
    root: &Path,
    sandbox: &Sandbox,
) {
    let glave_home = sandbox.glave_home.clone().unwrap();
    std::fs::write(glave_home.join("early"), "canary-early\n").unwrap();
    let mut command = Command::new(format!(
        "i=0; while [ ! -e go ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i + 1)); done; \
         test -e go && echo go-seen; cat '{home}/early' '{home}/late'; \
         mkdir -p \"$XDG_CACHE_HOME\" && echo cache-works > \"$XDG_CACHE_HOME/probe\" && \
         cat \"$XDG_CACHE_HOME/probe\"; true",
        home = glave_home.display()
    ));
    command.sandbox = Some(sandbox.clone());
    let started = computers.start(workspace, &command).await.unwrap();
    glave_ports::file::replace(&glave_home.join("early"), b"canary-replaced\n").unwrap();
    std::fs::write(glave_home.join("late"), "canary-late\n").unwrap();
    std::fs::write(root.join("go"), "").unwrap();
    let mut said = String::new();
    for _ in 0..3 {
        let wait = std::time::Duration::from_secs(60);
        let printed = computers.output(workspace, started, wait).await.unwrap();
        said.push_str(&printed.text);
        if printed.ended.is_some() {
            break;
        }
    }
    assert!(said.contains("go-seen"), "{said}");
    assert!(!said.contains("canary"), "{said}");
    assert!(said.contains("cache-works"), "{said}");
    for file in [
        glave_home.join("early"),
        glave_home.join("late"),
        root.join("go"),
    ] {
        std::fs::remove_file(file).unwrap();
    }
}

/// Prepared folders moved in whole under `.git/worktrees`, `.git/modules` (one that is there
/// and one nested in a submodule's), and `.glave`, and the project itself moved aside for a
/// prepared copy of it. `aside` is a folder outside the project the command may write, on the
/// project's volume ([`aside_folder`]).
fn moves_into_git(aside: &str) -> String {
    format!(
        "{PLANT} \
         plant {aside}/w/x; mv {aside}/w .git/worktrees; \
         plant {aside}/b; mv {aside}/b .git/modules/b; \
         mkdir -p .git/modules/a/modules && plant {aside}/c && mv {aside}/c .git/modules/a/modules/c; \
         mkdir -p {aside}/g && echo x > {aside}/g/settings.json && mv {aside}/g .glave; \
         root=$(pwd -P); mkdir -p {aside}/p && cp -R . {aside}/p && plant {aside}/p/.git && mv \"$root\" {aside}/old && mv {aside}/p \"$root\"; \
         echo done"
    )
}

/// What [`moves_into_git`] plants when nothing stops it.
const MOVED_INTO_GIT: &[&str] = &[
    ".git/worktrees/x/hooks/pre-commit",
    ".git/modules/b/hooks/pre-commit",
    ".git/modules/a/modules/c/hooks/pre-commit",
    ".glave/settings.json",
    ".git/hooks/pre-commit",
];

/// Seatbelt matches a rename by its two paths, never by what the moved folder holds, so
/// a prepared folder moved in whole must be stopped at its own name: nothing can be created
/// or moved under `.git/modules` or `.git/worktrees`, and the project cannot be swapped.
#[test]
fn a_sandboxed_repository_commits_but_nothing_moves_into_its_git_folder() {
    if !enforced() {
        return;
    }
    let dir = scratch("moves");
    let repo = committed_repo(&dir, "repo");
    std::fs::create_dir_all(repo.join(".git/modules/a")).unwrap();
    let (computers, sandbox) = computers(&dir);
    let aside = aside_folder(&sandbox);
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let path = repo.to_string_lossy().into_owned();
        let here = computers
            .open(computer.id, &Sources::Folder { path })
            .await
            .unwrap();
        git_works(&computers, here.id, &sandbox).await;
        let moves = moves_into_git(&format!("'{}'", aside.display()));
        let (_, said) = run(&computers, here.id, &sandbox, &moves).await;
        assert!(said.contains("done"), "{said}");
        let planted: Vec<&&str> = MOVED_INTO_GIT
            .iter()
            .filter(|planted| repo.join(planted).exists())
            .collect();
        assert!(planted.is_empty(), "planted {planted:?}: {said}");
        let prepared = [
            "w/x/hooks/pre-commit",
            "b/hooks/pre-commit",
            "g/settings.json",
        ];
        still_aside(&aside, &prepared, &said);
    });
    // Control: unsandboxed, the same moves plant every one, so the check above can fail.
    let open = committed_repo(&dir, "open");
    std::fs::create_dir_all(open.join(".git/modules/a")).unwrap();
    let aside = dir.join("aside");
    std::fs::create_dir_all(&aside).unwrap();
    let made = Process::new("bash")
        .args(["-c", &moves_into_git(&aside.to_string_lossy())])
        .current_dir(&open)
        .status()
        .unwrap();
    assert!(made.success());
    for planted in MOVED_INTO_GIT {
        assert!(open.join(planted).exists(), "{planted}");
    }
}

#[test]
fn a_sandboxed_worktree_commits_but_never_writes_the_main_checkout() {
    if !enforced() {
        return;
    }
    let dir = scratch("worktree");
    let repo = committed_repo(&dir, "app");
    let (computers, sandbox) = computers(&dir);
    let aside = aside_folder(&sandbox);
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let open = |branch: &str| Sources::Worktree {
            path: repo.to_string_lossy().into_owned(),
            branch: branch.into(),
            owner: None,
        };
        let mine = computers
            .open(computer.id, &open("glave/mine"))
            .await
            .unwrap();
        let other = computers
            .open(computer.id, &open("glave/other"))
            .await
            .unwrap();
        git_works(&computers, mine.id, &sandbox).await;

        let escapes = format!(
            "echo x > {main}/main.txt; echo x > {other}/theirs.txt; \
             echo x > {main}/.git/hooks/post-commit; echo x >> {main}/.git/config; true",
            main = repo.display(),
            other = other.root,
        );
        let (_, said) = run(&computers, mine.id, &sandbox, &escapes).await;
        assert!(!repo.join("main.txt").exists(), "{said}");
        assert!(
            !Path::new(&other.root).join("theirs.txt").exists(),
            "{said}"
        );
        assert!(!repo.join(".git/hooks/post-commit").exists(), "{said}");
        let config = std::fs::read_to_string(repo.join(".git/config")).unwrap();
        assert!(!config.ends_with("x\n"), "{config}");

        // Prepared folders moved in whole: under the shared folder's `worktrees` and
        // `modules`, under the worktree's own `modules`, and over its own folder and the
        // shared folder themselves.
        let common = repo.join(".git");
        let own = common.join("worktrees").join("glave-mine");
        assert!(own.is_dir(), "the worktree's own git folder");
        let moves = format!(
            "{PLANT} a='{aside}'; common='{common}'; own='{own}'; \
             plant $a/n; mv $a/n \"$common/worktrees/n\"; \
             plant $a/s/x; mv $a/s \"$common/modules\"; \
             plant $a/o/x; mv $a/o \"$own/modules\"; \
             cp -R \"$own\" $a/own && plant $a/own && mv \"$own\" $a/own-old && mv $a/own \"$own\"; \
             mkdir -p $a/c && cp -R \"$common/.\" $a/c && plant $a/c && mv \"$common\" $a/c-old && mv $a/c \"$common\"; \
             echo done",
            aside = aside.display(),
            common = common.display(),
            own = own.display(),
        );
        let (_, said) = run(&computers, mine.id, &sandbox, &moves).await;
        assert!(said.contains("done"), "{said}");
        let prepared = [
            "n/hooks/pre-commit",
            "s/x/hooks/pre-commit",
            "o/x/hooks/pre-commit",
        ];
        still_aside(&aside, &prepared, &said);
        let planted: Vec<PathBuf> = [
            common.join("worktrees/n/hooks/pre-commit"),
            common.join("modules/x/hooks/pre-commit"),
            own.join("modules/x/hooks/pre-commit"),
            own.join("hooks/pre-commit"),
            common.join("hooks/pre-commit"),
        ]
        .into_iter()
        .filter(|planted| planted.exists())
        .collect();
        assert!(planted.is_empty(), "planted {planted:?}: {said}");
        let commit = "echo four > d.txt && git add d.txt && \
            git -c user.name=t -c user.email=t@example.com commit -q -m four && echo committed";
        let (ending, said) = run(&computers, mine.id, &sandbox, commit).await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
    });
}

/// Spellings a command might use for a protected path, and the path a folder that ignores
/// case (APFS's default) takes each one to: case, a long s (folds to `s`) and the Kelvin sign
/// (folds to `k`).
const SPELLINGS: &[(&str, &str)] = &[
    (".git/hook\u{17f}/pre-commit", ".git/hooks/pre-commit"),
    (".GIT/hooks/post-checkout", ".git/hooks/post-checkout"),
    (".git/hoo\u{212a}s/post-merge", ".git/hooks/post-merge"),
    (".git/CONFIG", ".git/config"),
    (".GLAVE/settings.json", ".glave/settings.json"),
    (".v\u{17f}code/tasks.json", ".vscode/tasks.json"),
    (".mcp.j\u{17f}on", ".mcp.json"),
    ("ref\u{17f}/heads/main", "refs/heads/main"),
    (
        ".git/worktrees/w/hook\u{17f}/pre-commit",
        ".git/worktrees/w/hooks/pre-commit",
    ),
];

/// A script writing every spelling, making folders on the way.
fn write_spellings() -> String {
    let mut script = String::new();
    for (spelled, _) in SPELLINGS {
        let _ = write!(
            script,
            "mkdir -p \"$(dirname '{spelled}')\"; printf x > '{spelled}'; "
        );
    }
    script + "true"
}

fn ignores_case(dir: &Path) -> bool {
    std::fs::write(dir.join("case-probe"), "").unwrap();
    dir.join("CASE-PROBE").exists()
}

/// Seatbelt compares paths the way the folder does: on a folder that ignores case, a command
/// cannot reach a protected path through another spelling of it, whether the protected folder
/// exists yet or not. A name that only looks alike stays writable. Where there is no sandbox
/// (Linux, until bubblewrap lands), the command is refused and writes nothing.
#[test]
fn other_spellings_of_protected_paths_are_protected_too() {
    let enforced = enforced();
    let dir = scratch("spellings");
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect();
    let computers = LocalComputers::new(dir.join("copies"), environment);
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: false,
        home: None,
        glave_home: None,
    };
    let project = |name: &str, hooks: bool| {
        let root = dir.join(name);
        std::fs::create_dir_all(root.join(".git/worktrees/w")).unwrap();
        if hooks {
            std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        }
        root
    };
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        if !enforced {
            let root = project("unconfined", false);
            let path = root.to_string_lossy().into_owned();
            let here = computers
                .open(computer.id, &Sources::Folder { path })
                .await
                .unwrap();
            let mut command = Command::new(write_spellings());
            command.sandbox = Some(sandbox.clone());
            let never = CancelSwitch::default();
            let refused = computers
                .run(here.id, &command, &Collect::default(), &never)
                .await;
            assert!(
                matches!(refused, Err(ComputerError::Unsupported(_))),
                "{refused:?}"
            );
            for (spelled, protected) in SPELLINGS {
                assert!(!root.join(spelled).exists() && !root.join(protected).exists());
            }
            return;
        }
        for hooks in [false, true] {
            let root = project(&format!("sandboxed-{hooks}"), hooks);
            let path = root.to_string_lossy().into_owned();
            let here = computers
                .open(computer.id, &Sources::Folder { path })
                .await
                .unwrap();
            let script = write_spellings()
                + "; mkdir -p .git/look\u{17f} && printf x > .git/look\u{17f}/a && echo alike-ok";
            let (_, said) = run(&computers, here.id, &sandbox, &script).await;
            for (spelled, protected) in SPELLINGS {
                assert!(
                    !root.join(protected).exists(),
                    "{spelled} reached {protected}: {said}"
                );
            }
            assert!(said.contains("alike-ok"), "{said}");
        }
        // Control: unsandboxed, the same spellings land on the protected paths, so the check
        // above can fail.
        let root = project("open", false);
        if ignores_case(&root) {
            let made = Process::new("bash")
                .args(["-c", &write_spellings()])
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(made.success());
            for (spelled, protected) in SPELLINGS {
                assert!(
                    root.join(protected).exists(),
                    "{spelled} is not {protected} here"
                );
            }
        }
    });
}

/// Output that cancels its command as soon as anything arrives.
struct CancelOnOutput {
    said: Collect,
    switch: CancelSwitch,
}

impl Output for CancelOnOutput {
    fn write<'a>(&'a self, text: &'a str) -> glave_ports::BoxFuture<'a, ()> {
        self.switch.cancel();
        self.said.write(text)
    }
}

/// The temp folder a command printed, which it checked was there while it ran.
fn printed_temp(said: &str) -> PathBuf {
    let line = said.lines().find_map(|line| line.strip_prefix("temp="));
    let temp = PathBuf::from(line.unwrap_or_else(|| panic!("no temp folder in {said:?}")));
    assert!(temp.is_absolute(), "{said}");
    temp
}

/// Each command gets a temp folder of its own, gone once the command ends however it ends,
/// with read-only folders left in it (as Go's module cache makes).
#[test]
fn every_command_s_temp_folder_is_removed_when_it_ends() {
    if !enforced() {
        return;
    }
    let dir = scratch("run-temp");
    let (computers, sandbox) = computers(&dir);
    let prints = "test -d \"$TMPDIR\" && echo \"temp=$TMPDIR\"";
    block_on!(async {
        let here = open_app(&computers, &dir).await;
        let read_only = format!(
            "{prints} && mkdir -p \"$TMPDIR/mod/a\" && touch \"$TMPDIR/mod/a/f\" && \
             chmod -R a-w \"$TMPDIR/mod\""
        );
        let mut temps = Vec::new();
        for (script, ending) in [
            (read_only, Ending::Exited { code: 0 }),
            (format!("{prints}; exit 3"), Ending::Exited { code: 3 }),
        ] {
            let (ended, said) = run(&computers, here, &sandbox, &script).await;
            assert_eq!(ended, ending, "{said}");
            temps.push(printed_temp(&said));
        }
        // A one-second limit: the shortest a command can be given.
        let mut timed = Command::new(format!("{prints}; exec sleep 30"));
        timed.sandbox = Some(sandbox.clone());
        timed.timeout_secs = Some(1);
        let said = Collect::default();
        let finished = computers
            .run(here, &timed, &said, &CancelSwitch::default())
            .await
            .unwrap();
        let said = said.0.lock().unwrap().clone();
        assert_eq!(
            finished.ending,
            Ending::TimedOut { after_secs: 1 },
            "{said}"
        );
        temps.push(printed_temp(&said));
        let mut cancelled = Command::new(format!("{prints}; exec sleep 30"));
        cancelled.sandbox = Some(sandbox.clone());
        let output = CancelOnOutput {
            said: Collect::default(),
            switch: CancelSwitch::default(),
        };
        let finished = computers
            .run(here, &cancelled, &output, &output.switch)
            .await
            .unwrap();
        let said = output.said.0.lock().unwrap().clone();
        assert_eq!(finished.ending, Ending::Cancelled, "{said}");
        temps.push(printed_temp(&said));
        let left: Vec<&PathBuf> = temps.iter().filter(|temp| temp.exists()).collect();
        assert!(left.is_empty(), "left behind: {left:?}");
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// The person's keychains stay closed to a sandboxed command, with the network on, when the
/// security server's door is open for TLS roots (`specs/invariants.md`, 24): their folder
/// cannot be listed nor a keychain file read, and `security` finds no keychain to open. Under
/// the real home, since the keychains are found there; no item is read, inside or out.
#[cfg(target_os = "macos")]
#[test]
fn a_sandboxed_command_cannot_open_the_person_s_keychains() {
    if !enforced() {
        return;
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let keychains = home.join("Library/Keychains");
    let login = keychains.join("login.keychain-db");
    let dir = scratch("keychains");
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), home.clone().into_os_string())])
        .collect();
    let computers = LocalComputers::new(dir.join("copies"), environment);
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: true,
        home: Some(home),
        glave_home: Some(dir.join("glave")),
    };
    let script = format!(
        "ls '{0}' >/dev/null && echo keychain-listed; cat '{1}' >/dev/null && echo keychain-read; \
         security default-keychain -d user && echo keychain-opened; echo done",
        keychains.display(),
        login.display()
    );
    block_on!(async {
        let here = open_app(&computers, &dir).await;
        let (ending, said) = run(&computers, here, &sandbox, &script).await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        assert!(said.contains("done"), "{said}");
        for opened in ["keychain-listed", "keychain-read", "keychain-opened"] {
            assert!(!said.contains(opened), "{opened}: {said}");
        }
        // Refused by the sandbox, not merely missing: where each exists, outside it.
        for (path, there) in [(&keychains, keychains.is_dir()), (&login, login.is_file())] {
            let refused = format!("{}: Operation not permitted", path.display());
            assert!(!there || said.contains(&refused), "{refused}: {said}");
        }
    });
}
