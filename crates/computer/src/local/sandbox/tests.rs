//! A sandboxed worktree, under the real sandbox: it writes its own folder and its repository's
//! shared git folder, so `git commit` works, and nothing else of the repository's. Runs only
//! where the sandbox is (macOS), unless `GLAVE_REQUIRE_SANDBOX` says it must.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use glave_ports::{
    Command, Computers, Ending, Environment, Output, Sandbox, SandboxLevel, Sources, WorkspaceId,
};

use super::sandbox_available;
use crate::{LocalComputers, Switch};

fn enforced() -> bool {
    let available = sandbox_available();
    assert!(
        available || std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
        "GLAVE_REQUIRE_SANDBOX is set but the sandbox is not available here"
    );
    available
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// A folder outside the system temp folder, which the sandbox lets every command write.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sandbox-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::canonicalize(dir).unwrap()
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
        .run(workspace, &command, &output, &Switch::default())
        .await
        .unwrap();
    (finished.ending, output.0.lock().unwrap().clone())
}

#[test]
fn a_sandboxed_worktree_commits_but_never_writes_the_main_checkout() {
    if !enforced() {
        return;
    }
    let dir = scratch("worktree");
    let repo = dir.join("app");
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
        home: Some(home.clone()),
        glave_home: Some(glave_home),
    };
    block_on(async {
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

        let commit = "echo two > b.txt && git add b.txt && \
            git -c user.name=t -c user.email=t@example.com commit -q -m two && echo committed";
        let (ending, said) = run(&computers, mine.id, &sandbox, commit).await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        assert!(said.contains("committed"), "{said}");

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
    block_on(async {
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
            let refused = computers
                .run(here.id, &command, &Collect::default(), &Switch::default())
                .await;
            assert!(
                matches!(refused, Err(glave_ports::ComputerError::Unsupported(_))),
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
