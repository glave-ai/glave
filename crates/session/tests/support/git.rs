//! Git repositories for the tests that need one: a project's folder a worktree is made of.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `script` in `dir` with git as no one in particular, and checks it worked.
pub(crate) fn sh(dir: &Path, script: &str) {
    let ran = Command::new("bash")
        .args(["-c", script])
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(ran.success(), "{script}");
}

/// A repository at `dir/name` with one commit holding `AGENTS.md`.
pub(crate) fn repository(dir: &Path, name: &str) -> PathBuf {
    let repo = dir.join(name);
    std::fs::create_dir_all(&repo).unwrap();
    sh(
        &repo,
        "git init -q --initial-branch=main && echo 'Use tabs.' > AGENTS.md && git add . && \
         git -c user.name=t -c user.email=t@example.com commit -q -m one",
    );
    repo
}
