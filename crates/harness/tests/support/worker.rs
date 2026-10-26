//! What a worker thread starts from in the tests, and what it does: the repository it clones
//! and pushes to, and a turn that writes a file and hands one back.

use std::path::{Path, PathBuf};
use std::process::Command;

use glave_event::StopReason;
use glave_ports::Completion;
use serde_json::json;

use crate::support::answer;

/// A bare repository with one commit on `main`, as a worker clones it.
pub(crate) fn origin(dir: &Path) -> PathBuf {
    let origin = dir.join("origin.git");
    let script = format!(
        "git init -q --bare {o} && git init -q src && cd src && echo hi > a.txt && \
             git add a.txt && git -c user.name=t -c user.email=t@example.com commit -q -m one \
             && git push -q {o} HEAD:refs/heads/main",
        o = origin.display()
    );
    let made = Command::new("bash")
        .args(["-c", &script])
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(made.success());
    origin
}

/// The branches of `origin`, one per line.
pub(crate) fn branches(origin: &Path) -> String {
    let listed = Command::new("git")
        .args(["for-each-ref", "--format=%(refname:short)"])
        .current_dir(origin)
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout).into_owned()
}

/// A turn that writes `notes.txt` and `report.md` to hand back, then says "Done.".
pub(crate) fn hands_back() -> Vec<Completion> {
    vec![
        answer(
            None,
            vec![
                ("write", json!({"path": "notes.txt", "content": "work\n"})),
                (
                    "write",
                    json!({"path": ".glave/out/report.md", "content": "done\n"}),
                ),
            ],
            StopReason::ToolUse,
        ),
        answer(Some("Done."), vec![], StopReason::EndTurn),
    ]
}
