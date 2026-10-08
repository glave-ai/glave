//! Real commands under the real sandbox, without a model. They run only where the sandbox is
//! available (macOS); elsewhere preparing it must fail, never run unconfined.

use std::path::{Path, PathBuf};

use glave_event::Arguments;

use super::*;
use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// Whether to run the sandbox tests here. Where CI expects a sandbox (`GLAVE_REQUIRE_SANDBOX`),
/// its absence fails the test instead of skipping it, so a test never passes vacuously.
fn enforced() -> bool {
    let available = Sandbox::available();
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

/// A scratch folder outside the system temp folder, which the sandbox lets commands write.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sandbox-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("project")).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

fn sandbox(level: SandboxLevel, network: bool, home: &Path) -> Sandbox {
    Sandbox {
        level,
        network,
        home: Some(home.to_owned()),
        glave_home: Some(home.join(".glave")),
    }
}

/// Runs `command` with bash in `project`, under `sandbox` if one is given.
fn run_in(project: &Path, sandbox: Option<&Sandbox>, command: &str) -> crate::tools::ToolOutcome {
    let arguments =
        Arguments::Json(serde_json::from_value(serde_json::json!({ "command": command })).unwrap());
    block_on(async {
        let mut workspace = Workspace::open(project, std::env::vars_os()).await.unwrap();
        if let Some(sandbox) = sandbox {
            workspace = workspace.with_sandbox(sandbox).await.unwrap();
        }
        Tool::Bash.run(&arguments, &workspace).await
    })
}

fn run(project: &Path, sandbox: &Sandbox, command: &str) -> crate::tools::ToolOutcome {
    run_in(project, Some(sandbox), command)
}

#[test]
fn without_a_sandbox_on_this_computer_preparing_fails() {
    if Sandbox::available() {
        return;
    }
    let dir = scratch("unavailable");
    let prepared = block_on(sandbox(SandboxLevel::Project, false, &dir).prepare(&dir));
    assert!(prepared.is_err());
}

#[test]
fn writes_stay_in_the_project_and_temp_folder() {
    if !enforced() {
        return;
    }
    let dir = scratch("writes");
    let project = dir.join("project");
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        "echo in > inside.txt; echo out > ../outside.txt; echo t > \"$TMPDIR/glave-sbx-$$\" && rm \"$TMPDIR/glave-sbx-$$\" && echo temp-ok",
    );
    assert!(project.join("inside.txt").exists(), "{}", out.text);
    assert!(!dir.join("outside.txt").exists(), "{}", out.text);
    assert!(out.text.contains("temp-ok"), "{}", out.text);
    // Control: the same write without the sandbox escapes, so the check above can fail.
    run_in(&project, None, "echo out > ../outside.txt");
    assert!(dir.join("outside.txt").exists());

    let read_only = run(
        &project,
        &sandbox(SandboxLevel::ReadOnly, false, &dir),
        "echo x > read-only.txt",
    );
    assert!(!project.join("read-only.txt").exists());
    assert!(read_only.is_error);
    assert!(
        read_only.text.contains("blocked by the sandbox"),
        "{}",
        read_only.text
    );
}

#[test]
fn commits_work_but_hooks_config_and_settings_are_protected() {
    if !enforced() {
        return;
    }
    let dir = scratch("git");
    let project = dir.join("project");
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project)
        .status()
        .unwrap();
    assert!(init.success());
    let config_before = std::fs::read_to_string(project.join(".git/config")).unwrap();
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        "echo hi > a.txt && git add a.txt && git -c user.name=t -c user.email=t@example.com commit -qm first && echo committed; \
         echo 'echo pwned' > .git/hooks/pre-commit; git config core.hooksPath /tmp; mkdir .glave; echo '{}' > .mcp.json; true",
    );
    assert!(out.text.contains("committed"), "{}", out.text);
    assert!(
        !project.join(".git/hooks/pre-commit").exists(),
        "{}",
        out.text
    );
    assert_eq!(
        std::fs::read_to_string(project.join(".git/config")).unwrap(),
        config_before
    );
    assert!(!project.join(".glave").exists());
    assert!(!project.join(".mcp.json").exists());
}

#[test]
fn secret_locations_are_unreadable() {
    if !enforced() {
        return;
    }
    let dir = scratch("secrets");
    let project = dir.join("project");
    std::fs::create_dir_all(dir.join(".ssh")).unwrap();
    std::fs::write(dir.join(".ssh/id_ed25519"), "canary-ssh\n").unwrap();
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    std::fs::write(dir.join(".glave/glave.db"), "canary-db\n").unwrap();
    std::fs::write(project.join(".env"), "canary-env\n").unwrap();
    std::fs::write(project.join(".ENV.local"), "canary-env-local\n").unwrap();
    std::fs::write(project.join("server.pem"), "canary-pem\n").unwrap();
    std::fs::write(project.join(".env.example"), "example-ok\n").unwrap();
    std::fs::write(project.join("notes.md"), "notes-ok\n").unwrap();
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        &format!(
            "for f in {}/.ssh/id_ed25519 {}/.glave/glave.db .env .ENV.local server.pem .env.example notes.md; do cat \"$f\"; done; true",
            dir.display(),
            dir.display()
        ),
    );
    assert!(!out.text.contains("canary"), "{}", out.text);
    assert!(out.text.contains("example-ok"), "{}", out.text);
    assert!(out.text.contains("notes-ok"), "{}", out.text);
    // Control: without the sandbox every canary is readable.
    let open = run_in(
        &project,
        None,
        &format!(
            "cat {}/.ssh/id_ed25519 .env .ENV.local server.pem",
            dir.display()
        ),
    );
    assert_eq!(open.text.matches("canary").count(), 4, "{}", open.text);
}

#[test]
fn the_network_is_off_unless_allowed() {
    if !enforced() {
        return;
    }
    let dir = scratch("network");
    let project = dir.join("project");
    // A local listener only: no traffic leaves the machine.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connect = format!("exec 3<>/dev/tcp/127.0.0.1/{port} && echo connected");
    let off = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        &connect,
    );
    assert!(!off.text.contains("connected"), "{}", off.text);
    let on = run(
        &project,
        &sandbox(SandboxLevel::Project, true, &dir),
        &connect,
    );
    assert!(on.text.contains("connected"), "{}", on.text);
}

#[test]
fn everyday_tools_still_work() {
    if !enforced() {
        return;
    }
    let dir = scratch("tools");
    let out = run(
        &dir.join("project"),
        &sandbox(SandboxLevel::Project, false, &dir),
        "git --version >/dev/null && cargo --version >/dev/null && ls >/dev/null && python3 -c 'print(1)' >/dev/null && echo tools-ok",
    );
    assert!(out.text.contains("tools-ok"), "{}", out.text);
    assert!(!out.is_error, "{}", out.text);
}
