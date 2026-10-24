//! Real commands under the real sandbox, without a model. They run only where the sandbox is
//! available (macOS, and Linux with bubblewrap); elsewhere preparing it must fail, never run
//! unconfined.

use std::path::Path;

use glave_event::Arguments;
use glave_ports::block_on;

use glave_computer::sandbox_available;
use glave_ports::computer::{Sandbox, SandboxLevel, is_protected};

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

/// A scratch folder outside the system temp folder, which the sandbox lets commands write.
fn scratch(name: &str) -> glave_ports::testing::TempDir {
    let dir = glave_ports::testing::TempDir::outside_temp(name).unwrap();
    std::fs::create_dir_all(dir.join("project")).unwrap();
    dir
}

fn sandbox(level: SandboxLevel, network: bool, home: &Path) -> Sandbox {
    Sandbox {
        level,
        network,
        home: Some(home.to_owned()),
        glave_home: Some(home.join(".glave")),
    }
}

/// A `bash` call's arguments.
fn bash(arguments: serde_json::Value) -> Arguments {
    Arguments::Json(serde_json::from_value(arguments).unwrap())
}

/// Runs `command` with bash in `project`, under `sandbox` if one is given.
fn run_in(project: &Path, sandbox: Option<&Sandbox>, command: &str) -> crate::tools::ToolOutcome {
    let arguments = bash(serde_json::json!({ "command": command }));
    block_on!(async {
        let mut workspace = Workspace::open(project, std::env::vars_os()).await.unwrap();
        if let Some(sandbox) = sandbox {
            workspace = workspace.with_sandbox(sandbox).await.unwrap();
        }
        Tool::Bash
            .run(
                &arguments,
                (&workspace, &crate::background::Commands::default()),
                &Cancel::new(),
            )
            .await
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
    let prepared = block_on!(async {
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
fn a_command_allowed_outside_the_sandbox_runs_without_it_and_the_next_is_sandboxed_again() {
    if !enforced() {
        return;
    }
    let dir = scratch("outside");
    let project = dir.join("project");
    let sandbox = sandbox(SandboxLevel::Project, false, &dir);
    let blocked = run(&project, &sandbox, "echo out > ../outside.txt");
    assert!(!dir.join("outside.txt").exists(), "{}", blocked.text);
    assert!(
        blocked
            .text
            .contains("network: true when it needs the internet, or outside_sandbox: true"),
        "the note says how to ask: {}",
        blocked.text
    );
    let outside = bash(serde_json::json!({
        "command": "echo out > ../outside.txt",
        "outside_sandbox": true,
        "reason": "it writes beside the project",
    }));
    let again = bash(serde_json::json!({ "command": "echo again > ../again.txt" }));
    let (ran, after) = block_on!(async {
        let workspace = Workspace::open(&project, std::env::vars_os())
            .await
            .unwrap()
            .with_sandbox(&sandbox)
            .await
            .unwrap();
        let ran = Tool::Bash
            .run(
                &outside,
                (&workspace, &crate::background::Commands::default()),
                &Cancel::new(),
            )
            .await;
        let after = Tool::Bash
            .run(
                &again,
                (&workspace, &crate::background::Commands::default()),
                &Cancel::new(),
            )
            .await;
        (ran, after)
    });
    assert!(dir.join("outside.txt").exists(), "{}", ran.text);
    assert!(!dir.join("again.txt").exists(), "{}", after.text);
}

#[test]
fn running_outside_the_sandbox_is_added_to_a_command_s_own_caution() {
    use super::{OUTSIDE_SANDBOX, WITH_NETWORK, with_outside};
    use glave_event::Lifted;
    assert_eq!(with_outside(None, None), None);
    assert_eq!(
        with_outside(None, Some(Lifted::Sandbox)).as_deref(),
        Some(OUTSIDE_SANDBOX)
    );
    assert_eq!(
        with_outside(
            Some("it pushes to a remote".to_owned()),
            Some(Lifted::Sandbox)
        )
        .as_deref(),
        Some(
            "it pushes to a remote, and it runs outside the sandbox, where it can read and \
             change any file you can"
        )
    );
    assert_eq!(
        with_outside(
            Some("it pushes to a remote".to_owned()),
            Some(Lifted::Network)
        )
        .as_deref(),
        Some(
            "it pushes to a remote, and it can reach the internet, which commands here otherwise cannot"
        )
    );
    assert_eq!(
        with_outside(None, Some(Lifted::Network)).as_deref(),
        Some(WITH_NETWORK)
    );
    assert_eq!(
        with_outside(Some("it pushes to a remote".to_owned()), None).as_deref(),
        Some("it pushes to a remote")
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
    std::fs::write(project.join("privkey.pem"), "canary-pem\n").unwrap();
    std::fs::create_dir_all(dir.join(".config/gh")).unwrap();
    std::fs::write(dir.join(".config/gh/hosts.yml"), "canary-gh\n").unwrap();
    std::fs::create_dir_all(dir.join(".cargo")).unwrap();
    std::fs::write(dir.join(".cargo/credentials.toml"), "canary-cargo\n").unwrap();
    std::fs::write(dir.join(".cargo/config.toml"), "cargo-config-ok\n").unwrap();
    std::fs::write(dir.join(".zsh_history"), "canary-history\n").unwrap();
    // Public certificates and Java key stores are not hidden.
    std::fs::write(project.join("cert.pem"), "cert-ok\n").unwrap();
    std::fs::write(project.join("fullchain.pem"), "chain-ok\n").unwrap();
    std::fs::write(project.join("debug.keystore"), "keystore-ok\n").unwrap();
    std::fs::write(project.join(".env.example"), "example-ok\n").unwrap();
    std::fs::write(project.join("secrets.yaml"), "canary-yaml\n").unwrap();
    // Source modules named `secrets` are code, not secrets.
    std::fs::write(project.join("secrets.rs"), "rs-ok\n").unwrap();
    std::fs::write(project.join("secrets.py"), "py-ok\n").unwrap();
    std::fs::write(project.join("notes.md"), "notes-ok\n").unwrap();
    // A certificate bundle is public: Python's HTTPS needs certifi's in a project's venv.
    let certifi = project.join(".venv/lib/certifi");
    std::fs::create_dir_all(&certifi).unwrap();
    std::fs::write(certifi.join("cacert.pem"), "bundle-ok\n").unwrap();
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        &format!(
            "for f in {home}/.ssh/id_ed25519 {home}/.glave/glave.db {home}/.config/gh/hosts.yml {home}/.cargo/credentials.toml {home}/.cargo/config.toml {home}/.zsh_history .env .ENV.local privkey.pem secrets.yaml .env.example secrets.rs secrets.py notes.md .venv/lib/certifi/cacert.pem cert.pem fullchain.pem debug.keystore; do cat \"$f\"; done; true",
            home = dir.display(),
        ),
    );
    assert!(!out.text.contains("canary"), "{}", out.text);
    for open in ["cargo-config-ok", "cert-ok", "chain-ok", "keystore-ok"] {
        assert!(out.text.contains(open), "{open}: {}", out.text);
    }
    assert!(out.text.contains("example-ok"), "{}", out.text);
    assert!(out.text.contains("notes-ok"), "{}", out.text);
    assert!(out.text.contains("bundle-ok"), "{}", out.text);
    assert!(
        out.text.contains("rs-ok") && out.text.contains("py-ok"),
        "{}",
        out.text
    );
    // Control: without the sandbox every canary is readable.
    let open = run_in(
        &project,
        None,
        &format!(
            "cat {}/.ssh/id_ed25519 .env .ENV.local privkey.pem",
            dir.display()
        ),
    );
    assert_eq!(open.text.matches("canary").count(), 4, "{}", open.text);
}

/// A hidden file is refused, as on every platform, never read as empty: the command fails,
/// and the model is told the sandbox hides secrets.
#[test]
fn reading_a_hidden_file_fails_and_says_so() {
    if !enforced() {
        return;
    }
    let dir = scratch("hidden");
    let project = dir.join("project");
    std::fs::write(project.join(".env"), "canary-env\n").unwrap();
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, false, &dir),
        "cat .env",
    );
    assert!(out.is_error, "{}", out.text);
    assert!(!out.text.contains("canary"), "{}", out.text);
    let said = out.text.to_ascii_lowercase();
    assert!(
        said.contains("permission denied") || said.contains("operation not permitted"),
        "{}",
        out.text
    );
    assert!(out.text.contains("files such as .env"), "{}", out.text);
}

/// Package managers keep their caches in the home folder, which sandboxed commands cannot
/// write; in the sandbox they go in glave's sandbox cache folder, kept between runs, so
/// installing a dependency works and stays fast. The rest of glave's folder stays hidden.
#[test]
fn package_caches_are_in_glave_s_sandbox_cache_and_stay_warm() {
    if !enforced() {
        return;
    }
    let dir = scratch("caches");
    let project = dir.join("project");
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    std::fs::write(dir.join(".glave/glave.db"), "canary-db\n").unwrap();
    let cache = std::fs::canonicalize(dir.join(".glave"))
        .unwrap()
        .join("sandbox-cache");
    let sandbox = sandbox(SandboxLevel::Project, true, &dir);
    let variables = "XDG_CACHE_HOME npm_config_cache npm_config_store_dir YARN_CACHE_FOLDER \
        BUN_INSTALL_CACHE_DIR PIP_CACHE_DIR UV_CACHE_DIR UV_PYTHON_INSTALL_DIR POETRY_CACHE_DIR \
        PRE_COMMIT_HOME HF_HOME DENO_DIR GOPATH GOMODCACHE GOCACHE GRADLE_USER_HOME";
    let out = run(
        &project,
        &sandbox,
        &format!(
            "for v in {variables}; do case \"${{!v}}\" in {cache}/*) mkdir -p \"${{!v}}\" && \
             echo warm > \"${{!v}}/x\" && echo \"$v writable\";; *) echo \"$v is ${{!v}}\";; esac; done; \
             echo \"maven: $MAVEN_OPTS\"; cat {glave}/glave.db",
            cache = cache.display(),
            glave = dir.join(".glave").display(),
        ),
    );
    assert_eq!(out.text.matches("writable").count(), 16, "{}", out.text);
    assert!(
        out.text
            .contains(&format!("-Dmaven.repo.local={}/maven", cache.display())),
        "{}",
        out.text
    );
    assert!(!out.text.contains("canary"), "{}", out.text);
    // The next run, in a sandbox prepared afresh, finds what the last one left.
    let again = run(&project, &sandbox, "cat \"$PIP_CACHE_DIR/x\"");
    assert!(again.text.contains("warm"), "{}", again.text);
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
    assert!(
        off.text.contains("network: true"),
        "the note says how to ask: {}",
        off.text
    );
    let on = run(
        &project,
        &sandbox(SandboxLevel::Project, true, &dir),
        &connect,
    );
    assert!(on.text.contains("connected"), "{}", on.text);
    // Let out with the network, one command keeps the rest of the sandbox.
    let with_network = bash(serde_json::json!({
        "command": format!("echo out > ../outside.txt; {connect}"),
        "network": true,
    }));
    let lifted = block_on!(async {
        let workspace = Workspace::open(&project, std::env::vars_os())
            .await
            .unwrap()
            .with_sandbox(&sandbox(SandboxLevel::Project, false, &dir))
            .await
            .unwrap();
        Tool::Bash
            .run(
                &with_network,
                (&workspace, &crate::background::Commands::default()),
                &Cancel::new(),
            )
            .await
    });
    assert!(lifted.text.contains("connected"), "{}", lifted.text);
    assert!(!dir.join("outside.txt").exists(), "{}", lifted.text);
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
        block_on!(async {
            let mut workspace = Workspace::open(&project, std::env::vars_os())
                .await
                .unwrap();
            if sandboxed {
                let sandbox = sandbox(SandboxLevel::Project, false, &dir);
                workspace = workspace.with_sandbox(&sandbox).await.unwrap();
            }
            Tool::Write
                .run(
                    &arguments,
                    (&workspace, &crate::background::Commands::default()),
                    &Cancel::new(),
                )
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
        block_on!(async {
            let workspace = Workspace::open(&project, std::env::vars_os())
                .await
                .unwrap()
                .with_sandbox(&sandbox)
                .await
                .unwrap();
            tool.run(
                &arguments,
                (&workspace, &crate::background::Commands::default()),
                &Cancel::new(),
            )
            .await
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

/// A crate with a `secrets` module builds and tests in the default sandbox: the name rule
/// hides secret data files, never source.
#[test]
fn a_crate_with_a_secrets_module_builds_and_tests_in_the_sandbox() {
    if !enforced() {
        return;
    }
    let dir = scratch("secrets-module");
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"vault\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(project.join("src/lib.rs"), "pub mod secrets;\n").unwrap();
    std::fs::write(
        project.join("src/secrets.rs"),
        "pub fn masked() -> &'static str { \"***\" }\n\n#[test]\nfn masks() { assert_eq!(masked(), \"***\"); }\n",
    )
    .unwrap();
    let out = run(
        &project,
        &sandbox(SandboxLevel::Project, true, &dir),
        "CARGO_TARGET_DIR=target cargo test --offline --quiet 2>&1 && echo tested",
    );
    assert!(out.text.contains("tested"), "{}", out.text);
}
