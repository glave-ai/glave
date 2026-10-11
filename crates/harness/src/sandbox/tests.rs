//! Real commands under the real sandbox, without a model. They run only where the sandbox is
//! available (macOS, and Linux with bubblewrap); elsewhere preparing it must fail, never run
//! unconfined.

use std::path::{Path, PathBuf};

use glave_event::Arguments;

use glave_computer::sandbox_available;
use glave_ports::{Sandbox, SandboxLevel, is_protected};

use crate::cancel::Cancel;
use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// Whether to run the sandbox tests here. Where CI expects a sandbox (`GLAVE_REQUIRE_SANDBOX`),
/// its absence fails the test instead of skipping it, so a test never passes vacuously.
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
        Tool::Bash.run(&arguments, &workspace, &Cancel::new()).await
    })
}

fn run(project: &Path, sandbox: &Sandbox, command: &str) -> crate::tools::ToolOutcome {
    run_in(project, Some(sandbox), command)
}

#[test]
fn without_a_sandbox_on_this_computer_preparing_fails() {
    if sandbox_available() {
        return;
    }
    let dir = scratch("unavailable");
    let prepared = block_on(async {
        Workspace::open(&dir, std::env::vars_os())
            .await
            .unwrap()
            .with_sandbox(&sandbox(SandboxLevel::Project, false, &dir))
            .await
    });
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
    // A certificate bundle is public: Python's HTTPS needs certifi's in a project's venv.
    let certifi = project.join(".venv/lib/certifi");
    std::fs::create_dir_all(&certifi).unwrap();
    std::fs::write(certifi.join("cacert.pem"), "bundle-ok\n").unwrap();
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        &format!(
            "for f in {}/.ssh/id_ed25519 {}/.glave/glave.db .env .ENV.local server.pem .env.example notes.md .venv/lib/certifi/cacert.pem; do cat \"$f\"; done; true",
            dir.display(),
            dir.display()
        ),
    );
    assert!(!out.text.contains("canary"), "{}", out.text);
    assert!(out.text.contains("example-ok"), "{}", out.text);
    assert!(out.text.contains("notes-ok"), "{}", out.text);
    assert!(out.text.contains("bundle-ok"), "{}", out.text);
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

/// Package managers keep their caches in the home folder, which sandboxed commands cannot
/// write; in the sandbox they go in its temp folder, so installing a dependency works.
#[test]
fn package_caches_are_in_the_sandbox_s_temp_folder() {
    if !enforced() {
        return;
    }
    let dir = scratch("caches");
    let project = dir.join("project");
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, true, &dir),
        "for v in XDG_CACHE_HOME npm_config_cache YARN_CACHE_FOLDER PIP_CACHE_DIR UV_CACHE_DIR \
         GOPATH GOMODCACHE GOCACHE; do case \"${!v}\" in \"$TMPDIR\"/*) mkdir -p \"${!v}\" && \
         touch \"${!v}/x\" && echo \"$v writable\";; *) echo \"$v is ${!v}\";; esac; done",
    );
    assert_eq!(out.text.matches("writable").count(), 8, "{}", out.text);
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

#[test]
fn protected_paths_are_recognized_in_any_case() {
    let root = Path::new("/p");
    for path in [
        ".git",
        ".git/hooks/pre-commit",
        ".GIT/Config",
        ".git/config.worktree",
        ".git/worktrees/w/config",
        ".git/modules/m/hooks/post-checkout",
        ".glave",
        ".glave/settings.json",
        "HEAD",
        "refs/heads/main",
        ".mcp.json",
        ".vscode/tasks.json",
        ".envrc",
        ".git/hook\u{17f}/pre-commit",
        ".V\u{17f}CODE/tasks.json",
        ".mcp.j\u{17f}on",
    ] {
        assert!(is_protected(root, &root.join(path)), "{path}");
    }
    for path in [
        ".git/index",
        ".git/objects/ab/cdef",
        ".git/refs/heads/main",
        ".git/worktrees/w/HEAD",
        ".gitignore",
        ".github/workflows/ci.yml",
        "head.txt",
        "src/hooks/use.ts",
    ] {
        assert!(!is_protected(root, &root.join(path)), "{path}");
    }
    assert!(!is_protected(root, Path::new("/elsewhere/.git/hooks")));
}

#[test]
fn the_file_tools_refuse_protected_paths_in_a_sandbox() {
    if !enforced() {
        return;
    }
    let dir = scratch("file-tools");
    let project = dir.join("project");
    std::fs::create_dir_all(project.join(".git/hooks")).unwrap();
    std::fs::write(project.join(".git/hooks/existing"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        project.join(".git/hooks/existing"),
        project.join("notes.txt"),
    )
    .unwrap();
    let write = |sandboxed: bool, path: &str| {
        let arguments = Arguments::Json(
            serde_json::from_value(serde_json::json!({ "path": path, "content": "echo pwned\n" }))
                .unwrap(),
        );
        block_on(async {
            let mut workspace = Workspace::open(&project, std::env::vars_os())
                .await
                .unwrap();
            if sandboxed {
                let sandbox = sandbox(SandboxLevel::Project, false, &dir);
                workspace = workspace.with_sandbox(&sandbox).await.unwrap();
            }
            Tool::Write
                .run(&arguments, &workspace, &Cancel::new())
                .await
        })
    };
    let refused = write(true, ".git/hooks/pre-commit");
    assert!(refused.is_error && refused.text.contains("protected by the sandbox"));
    assert!(!project.join(".git/hooks/pre-commit").exists());
    let through_link = write(true, "notes.txt");
    assert!(
        through_link.is_error && through_link.text.contains("protected by the sandbox"),
        "{}",
        through_link.text
    );
    assert_eq!(
        std::fs::read_to_string(project.join(".git/hooks/existing")).unwrap(),
        "#!/bin/sh\n"
    );
    // Control: without the sandbox the same write goes through, so the refusal is the sandbox's.
    let open = write(false, ".git/hooks/pre-commit");
    assert!(!open.is_error, "{}", open.text);
    assert!(project.join(".git/hooks/pre-commit").exists());
}

/// Whether the folder ignores case in names, as macOS folders usually do.
fn ignores_case(dir: &Path) -> bool {
    std::fs::write(dir.join("case-probe"), "").unwrap();
    dir.join("CASE-PROBE").exists()
}

/// Paths through the links the test plants, and, where the folder ignores case, other
/// spellings of protected names.
fn planted_paths(project: &Path) -> Vec<&'static str> {
    let mut paths = vec![
        "hooks/pre-commit",
        "settings/settings.json",
        "settings/new.json",
        "conf",
        "git_dir/config",
        "git_dir/hooks/post-checkout",
    ];
    if ignores_case(project) {
        paths.extend([
            ".GIT/hooks/pre-commit",
            ".Git/config",
            ".GLAVE/settings.json",
            ".git/hook\u{17f}/pre-commit",
            "HOOKS/pre-commit",
        ]);
    }
    paths
}

/// A command in the sandbox may plant links in the project; the file tools still refuse to
/// write or edit what they lead to, as they refuse spellings the folder treats as the same
/// protected name.
#[test]
fn the_file_tools_refuse_links_a_command_planted_into_protected_paths() {
    if !enforced() {
        return;
    }
    let dir = scratch("planted-links");
    let project = dir.join("project");
    std::fs::create_dir_all(project.join(".git/hooks")).unwrap();
    std::fs::create_dir_all(project.join(".glave")).unwrap();
    std::fs::write(project.join(".glave/settings.json"), "{}\n").unwrap();
    let sandbox = sandbox(SandboxLevel::Project, false, &dir);
    let planted = run(
        &project,
        &sandbox,
        "ln -s .git/hooks hooks && ln -s .glave settings && ln -s .glave/settings.json conf && \
         ln -s .git git_dir && echo planted",
    );
    assert!(planted.text.contains("planted"), "{}", planted.text);
    let tool = |tool: Tool, arguments: serde_json::Value| {
        let arguments = Arguments::Json(serde_json::from_value(arguments).unwrap());
        block_on(async {
            let workspace = Workspace::open(&project, std::env::vars_os())
                .await
                .unwrap()
                .with_sandbox(&sandbox)
                .await
                .unwrap();
            tool.run(&arguments, &workspace, &Cancel::new()).await
        })
    };
    for path in planted_paths(&project) {
        let write = tool(
            Tool::Write,
            serde_json::json!({ "path": path, "content": "echo pwned\n" }),
        );
        assert!(
            write.is_error && write.text.contains("protected by the sandbox"),
            "write {path}: {}",
            write.text
        );
        let edit = tool(
            Tool::Edit,
            serde_json::json!({ "path": path, "old_string": "{}", "new_string": "{\"x\": 1}" }),
        );
        assert!(
            edit.is_error && edit.text.contains("protected by the sandbox"),
            "edit {path}: {}",
            edit.text
        );
    }
    assert_eq!(
        std::fs::read_dir(project.join(".git/hooks"))
            .unwrap()
            .count(),
        0,
        "no hook was written"
    );
    assert!(!project.join(".git/config").exists());
    assert_eq!(
        std::fs::read_to_string(project.join(".glave/settings.json")).unwrap(),
        "{}\n"
    );
    // Control: through a link to an ordinary folder, the same write goes through.
    std::fs::create_dir_all(project.join("docs")).unwrap();
    std::os::unix::fs::symlink("docs", project.join("docs_link")).unwrap();
    let ordinary = tool(
        Tool::Write,
        serde_json::json!({ "path": "docs_link/notes.txt", "content": "fine\n" }),
    );
    assert!(!ordinary.is_error, "{}", ordinary.text);
    assert_eq!(
        std::fs::read_to_string(project.join("docs/notes.txt")).unwrap(),
        "fine\n"
    );
}

/// The bug the live evals found: a project inside a writable temp folder lost its protections,
/// because the temp grant covered it. Checked in both temp folders commands may write.
#[test]
fn a_project_inside_a_temp_folder_keeps_its_protections() {
    if !enforced() {
        return;
    }
    let system_temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let shared_tmp = std::fs::canonicalize("/tmp").unwrap();
    for base in [system_temp, shared_tmp] {
        let dir = base.join(format!("glave-sbx-in-temp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap();
        assert!(init.success());
        let out = run(
            &project,
            &sandbox(SandboxLevel::Project, false, &dir),
            "echo 'echo pwned' > .git/hooks/pre-commit; echo t > \"$TMPDIR/ok\" && echo temp-ok; true",
        );
        assert!(
            !project.join(".git/hooks/pre-commit").exists(),
            "{base:?}: {}",
            out.text
        );
        assert!(out.text.contains("temp-ok"), "{}", out.text);
        let read_only = run(
            &project,
            &sandbox(SandboxLevel::ReadOnly, false, &dir),
            "echo x > inside.txt; true",
        );
        assert!(
            !project.join("inside.txt").exists(),
            "{base:?}: {}",
            read_only.text
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn builds_with_dependencies_work_offline_in_the_sandbox() {
    if !enforced() {
        return;
    }
    let dir = scratch("deps");
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"deps\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nserde = \"1\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(project.join("src/lib.rs"), "pub use serde;\n").unwrap();
    let build = "CARGO_TARGET_DIR=target cargo build --offline --quiet && echo built";
    // Control: outside the sandbox. Without serde in the local cache there is nothing to test.
    let open = run_in(&project, None, build);
    if !open.text.contains("built") {
        eprintln!(
            "skipped: serde is not in this machine's cargo cache: {}",
            open.text
        );
        return;
    }
    std::fs::remove_dir_all(project.join("target")).unwrap();
    let sandboxed = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        build,
    );
    assert!(sandboxed.text.contains("built"), "{}", sandboxed.text);
}
