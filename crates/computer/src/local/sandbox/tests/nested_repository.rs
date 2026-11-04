//! A repository inside the project, under the real sandbox.

use super::*;

/// A repository inside the project is the command's to work in, config and all, and to make; so
/// what it plants there is proof against glave's own git, not against the command.
#[test]
fn a_nested_repository_s_config_is_the_command_s_to_write_and_runs_nothing_for_glave() {
    if !enforced() {
        return;
    }
    let dir = scratch("sandbox-nested-repository");
    let (computers, sandbox) = computers(&dir);
    let nested = committed_repo(&dir, "app/nested");
    let ran = dir.join("ran");
    let program = dir.join("program");
    std::fs::write(&program, format!("#!/bin/sh\n: > '{}'\n", ran.display())).unwrap();
    let script = format!(
        "export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com \
         GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com && \
         git init -q made && git clone -q nested cloned && \
         git -C made commit -q --allow-empty -m one && \
         git -C nested remote add origin https://example.invalid/app.git && \
         git -C nested worktree add -q ../beside -b beside && \
         git -C nested config core.fsmonitor '{program}' && \
         printf '#!/bin/sh\\n{program}\\n' > nested/.git/hooks/post-checkout && \
         chmod +x nested/.git/hooks/post-checkout && echo planted",
        program = program.display()
    );
    block_on!(async {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let workspace = open_app(&computers, &dir).await;
        let (ending, said) = run(&computers, workspace, &sandbox, &script).await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        assert!(said.contains("planted"), "{said}");
        let computer = computers.create(&Environment::host()).await.unwrap();
        let worktree = Sources::Worktree {
            path: nested.to_string_lossy().into_owned(),
            branch: "glave/thread".into(),
            owner: None,
        };
        computers.open(computer.id, &worktree).await.unwrap();
    });
    assert!(!ran.exists(), "glave's git ran what the command planted");
}
