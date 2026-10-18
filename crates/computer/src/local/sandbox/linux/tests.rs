//! Real commands in the real Linux sandbox. The bubblewrap tests run where bubblewrap starts,
//! and must where `GLAVE_REQUIRE_SANDBOX` is set; the Landlock tests run where it does not
//! but Landlock and a helper (`GLAVE_TEST_SANDBOX_HELPER`, a built glave) are there, and must
//! where `GLAVE_REQUIRE_LANDLOCK` is set. ext4 tells cases apart, so only the exact spellings
//! of protected names are protected, and those are what these check.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use glave_ports::{
    Command, ComputerError, Computers, Ending, Environment, Output, Sandbox, SandboxLevel, Sources,
    WorkspaceId, block_on,
};

use super::landlocked;
use crate::local::sandbox_available;
use crate::{CancelSwitch, LocalComputers};

fn bubblewrap() -> bool {
    let available = sandbox_available();
    assert!(
        available || std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
        "GLAVE_REQUIRE_SANDBOX is set but bubblewrap cannot start here"
    );
    available
}

/// The helper to run the Landlock tests with, where they run.
fn landlock() -> Option<PathBuf> {
    let helper = std::env::var_os("GLAVE_TEST_SANDBOX_HELPER").map(PathBuf::from);
    let runs = !sandbox_available() && landlocked::available() && helper.is_some();
    assert!(
        runs || std::env::var_os("GLAVE_REQUIRE_LANDLOCK").is_none(),
        "GLAVE_REQUIRE_LANDLOCK is set but bubblewrap starts, Landlock is missing, or \
         GLAVE_TEST_SANDBOX_HELPER is unset"
    );
    helper.filter(|_| runs)
}

/// A folder outside the system temp folder, which the sandbox lets every command write.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sandbox-tests")
        .join(format!("linux-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("project")).unwrap();
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

/// Runs `script` in `dir/project`, in a sandbox with `dir` as the home folder, or none.
fn run(
    dir: &Path,
    sandbox: Option<(SandboxLevel, bool)>,
    helper: Option<&Path>,
    script: &str,
) -> Result<(Ending, String), ComputerError> {
    let environment: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([("HOME".into(), dir.as_os_str().to_owned())])
        .collect();
    let mut computers = LocalComputers::new(dir.join("copies"), environment);
    if let Some(helper) = helper {
        computers = computers.with_sandbox_helper(helper);
    }
    let mut command = Command::new(script);
    command.sandbox = sandbox.map(|(level, network)| Sandbox {
        level,
        network,
        home: Some(dir.to_owned()),
        glave_home: Some(dir.join(".glave")),
    });
    block_on!(async {
        let computer = computers.create(&Environment::host()).await?;
        let sources = Sources::Folder {
            path: dir.join("project").to_string_lossy().into_owned(),
        };
        let workspace: WorkspaceId = computers.open(computer.id, &sources).await?.id;
        let output = Collect::default();
        let finished = computers
            .run(workspace, &command, &output, &CancelSwitch::default())
            .await?;
        Ok((finished.ending, output.0.lock().unwrap().clone()))
    })
}

fn git_init(project: &Path) {
    let made = Process::new("git")
        .args(["init", "-q"])
        .current_dir(project)
        .status()
        .unwrap();
    assert!(made.success());
}

#[test]
fn bubblewrap_keeps_protected_paths_read_only_even_moved_or_not_there_yet() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("protected");
    let project = dir.join("project");
    git_init(&project);
    std::fs::create_dir_all(project.join(".git/worktrees/w")).unwrap();
    std::fs::create_dir_all(project.join(".git/modules/m/hooks")).unwrap();
    std::fs::write(project.join(".git/hooks/existing"), "#!/bin/sh\n").unwrap();
    let config = std::fs::read_to_string(project.join(".git/config")).unwrap();
    let attempts = [
        "echo x > .git/hooks/pre-commit",
        "echo x >> .git/config",
        "echo x > .git/config.worktree",
        "echo x > .git/worktrees/w/config",
        "mkdir -p .git/worktrees/new && echo x > .git/worktrees/new/config",
        "echo x > .git/modules/m/hooks/post-checkout",
        "mkdir -p .glave && echo x > .glave/settings.json",
        "echo x > .mcp.json",
        "mkdir -p .vscode && echo x > .vscode/tasks.json",
        "echo x > .envrc",
        "echo x > HEAD",
        "mv .git/hooks moved-hooks",
        "mv .git moved-git && mkdir -p .git/hooks && echo x > .git/hooks/pre-commit",
        "rm -rf .git/hooks",
    ];
    let script =
        attempts.join(" ; ") + " ; echo x > ordinary.txt && echo x > .git/scratch && echo done";
    let (_, said) = run(&dir, Some((SandboxLevel::Project, false)), None, &script).unwrap();
    assert!(said.contains("done"), "{said}");
    assert!(project.join("ordinary.txt").exists(), "{said}");
    assert!(
        project.join(".git/scratch").exists(),
        "the rest of .git is writable: {said}"
    );
    // Moving the hooks only copies them (a mount point cannot move); the originals stay.
    assert!(project.join(".git/hooks/existing").exists(), "{said}");
    assert!(!project.join(".git/hooks/pre-commit").exists(), "{said}");
    assert_eq!(
        std::fs::read_to_string(project.join(".git/config")).unwrap(),
        config
    );
    for gone in [
        ".git/config.worktree",
        ".git/worktrees/w/config",
        ".git/worktrees/new",
        ".git/modules/m/hooks/post-checkout",
        ".glave",
        ".mcp.json",
        ".vscode",
        ".envrc",
        "HEAD",
        "moved-git",
    ] {
        assert!(!project.join(gone).exists(), "{gone}: {said}");
    }
    // Control: without the sandbox the same hook is written, so the check above can fail.
    run(&dir, None, None, "echo x > .git/hooks/pre-commit").unwrap();
    assert!(project.join(".git/hooks/pre-commit").exists());
}

#[test]
fn bubblewrap_refuses_a_protected_path_that_is_a_link() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("link");
    std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("project/.vscode")).unwrap();
    let refused = run(&dir, Some((SandboxLevel::Project, false)), None, "true");
    assert!(
        matches!(&refused, Err(ComputerError::Refused(why)) if why.contains(".vscode")),
        "{refused:?}"
    );
}

#[test]
fn bubblewrap_blocks_unix_sockets_and_ptrace() {
    if !bubblewrap() {
        return;
    }
    let dir = scratch("seccomp");
    let socket = dir.join("outside.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let connect = format!(
        "python3 -c \"import socket; s = socket.socket(socket.AF_UNIX); s.connect('{}'); print('connected')\"",
        socket.display()
    );
    for network in [false, true] {
        let (_, said) = run(&dir, Some((SandboxLevel::Project, network)), None, &connect).unwrap();
        assert!(!said.contains("connected"), "network {network}: {said}");
    }
    let (_, open) = run(&dir, None, None, &connect).unwrap();
    assert!(open.contains("connected"), "{open}");
    let ptrace =
        "python3 -c \"import ctypes; print('ptrace', ctypes.CDLL(None).ptrace(0, 0, 0, 0))\"";
    let (_, said) = run(&dir, Some((SandboxLevel::Project, true)), None, ptrace).unwrap();
    assert!(said.contains("ptrace -1"), "{said}");
}

#[test]
fn landlock_confines_read_only_commands() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock");
    let project = dir.join("project");
    std::fs::create_dir_all(dir.join(".ssh")).unwrap();
    std::fs::write(dir.join(".ssh/id_ed25519"), "canary-ssh\n").unwrap();
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    std::fs::write(dir.join(".glave/glave.db"), "canary-db\n").unwrap();
    std::fs::write(project.join(".env"), "canary-env\n").unwrap();
    std::fs::write(project.join("notes.md"), "notes-ok\n").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!(
        "cat {home}/.ssh/id_ed25519 {home}/.glave/glave.db .env notes.md; \
         echo x > inside.txt; echo x > {home}/outside.txt; \
         echo t > \"$TMPDIR/t\" && echo temp-ok; echo t > /tmp/glave-landlock-$$ && rm /tmp/glave-landlock-$$ && echo tmp-ok; \
         (exec 3<>/dev/tcp/127.0.0.1/{port}) && echo connected; true",
        home = dir.display()
    );
    let (_, said) = run(
        &dir,
        Some((SandboxLevel::ReadOnly, false)),
        Some(&helper),
        &script,
    )
    .unwrap();
    assert!(!said.contains("canary"), "{said}");
    assert!(said.contains("notes-ok"), "{said}");
    assert!(
        said.contains("temp-ok") && said.contains("tmp-ok"),
        "{said}"
    );
    assert!(!said.contains("connected"), "{said}");
    assert!(!project.join("inside.txt").exists(), "{said}");
    assert!(!dir.join("outside.txt").exists(), "{said}");
    // Control: unconfined, every canary is readable and the write lands.
    let (_, open) = run(&dir, None, None, &script).unwrap();
    assert_eq!(open.matches("canary").count(), 3, "{open}");
    assert!(dir.join("outside.txt").exists());
}

#[test]
fn landlock_refuses_the_project_level_and_says_what_to_install() {
    let Some(helper) = landlock() else {
        return;
    };
    let dir = scratch("landlock-project");
    let refused = run(
        &dir,
        Some((SandboxLevel::Project, false)),
        Some(&helper),
        "true",
    );
    assert!(
        matches!(&refused, Err(ComputerError::Unsupported(why)) if why.contains("install bubblewrap")),
        "{refused:?}"
    );
}
