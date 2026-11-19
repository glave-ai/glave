//! Someone else's commands on the person's computer share no `/tmp` with the person's (#1583):
//! on Linux each gets one of its own, on macOS none at all.

use std::path::PathBuf;

use glave_ports::block_on;
use glave_ports::computer::{RunsFor, Sandbox};

use super::{computers, enforced, open_app, run, scratch};

/// Files in `/tmp` of this test's own, removed when dropped.
struct InTmp(Vec<PathBuf>);

impl Drop for InTmp {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// What the person's command leaves in `/tmp` someone else's never reads, and what someone
/// else's leaves there the person's next command never finds, to read or run.
#[test]
fn someone_else_s_commands_share_no_tmp_with_the_person_s() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-tmp");
    let name = format!("glave-test-own-tmp-{}", std::process::id());
    let (owners, theirs) = (format!("/tmp/{name}.owner"), format!("/tmp/{name}.theirs"));
    let _left = InTmp(vec![PathBuf::from(&owners), PathBuf::from(&theirs)]);
    let said = block_on!(async {
        let (computers, owner) = computers(&dir);
        let someone = Sandbox {
            runs_for: RunsFor::SomeoneElse,
            ..owner.clone()
        };
        let there = open_app(&computers, &dir).await;
        let mut said = Vec::new();
        for (sandbox, script) in [
            (
                &owner,
                format!("echo the-owner-s > '{owners}' && echo left"),
            ),
            (
                &someone,
                format!(
                    "cat '{owners}' 2>/dev/null || echo read-nothing; \
                     echo planted > '{theirs}' 2>/dev/null; true"
                ),
            ),
            (
                &owner,
                format!("cat '{theirs}' 2>/dev/null || echo found-nothing"),
            ),
        ] {
            said.push(run(&computers, there, sandbox, &script).await.1);
        }
        said
    });
    assert!(said[0].contains("left"), "the person's: {}", said[0]);
    assert!(said[1].contains("read-nothing"), "{}", said[1]);
    assert!(!said[1].contains("the-owner-s"), "{}", said[1]);
    assert!(said[2].contains("found-nothing"), "{}", said[2]);
}
