//! A sandboxed worktree, under the real sandbox: it writes its own folder and its repository's
//! shared git folder, so `git commit` works, and nothing else of the repository's. Runs only
//! where the sandbox is (macOS), unless `GLAVE_REQUIRE_SANDBOX` says it must.

use std::ffi::OsString;
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
