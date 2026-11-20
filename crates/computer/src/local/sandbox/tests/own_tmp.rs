//! Someone else's commands on the person's computer share no `/tmp` with the person's (#1583):
//! on Linux each gets one of its own, on macOS none at all; and they read nothing the person
//! left in `/var/tmp` or, on macOS, in the person's own temp folder.

use std::path::PathBuf;

use glave_ports::block_on;
use glave_ports::computer::{RunsFor, Sandbox};

use super::{computers, enforced, open_app, run, scratch};

/// Files of this test's own, removed when dropped.
struct Left(Vec<PathBuf>);

impl Drop for Left {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// What the person's command leaves in `/tmp` someone else's never reads, and what someone
/// else's leaves there the person's next command never finds, to read or run. On Linux
/// someone else's command still writes a `/tmp`, its own; nor does it read what the person
/// left in `/var/tmp` or the person's own temp folder, which the person's commands read.
#[test]
fn someone_else_s_commands_share_no_tmp_with_the_person_s() {
    if !enforced() {
        return;
    }
    let dir = scratch("own-tmp");
    let name = format!("glave-test-own-tmp-{}", std::process::id());
    let (owners, theirs) = (format!("/tmp/{name}.owner"), format!("/tmp/{name}.theirs"));
    // The person's own temp folder is `/tmp` itself on Linux, and its own on macOS.
    let mut elsewhere = vec![PathBuf::from("/var/tmp").join(&name)];
    if cfg!(target_os = "macos") {
        let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        elsewhere.push(temp.join(&name));
    }
    for file in &elsewhere {
        std::fs::write(file, "the-person-s-elsewhere\n").unwrap();
    }
    let mut left = vec![PathBuf::from(&owners), PathBuf::from(&theirs)];
    left.extend(elsewhere.iter().cloned());
    let _left = Left(left);
    let read_elsewhere = elsewhere
        .iter()
        .map(|file| format!("cat '{}' 2>/dev/null;", file.display()))
        .collect::<String>();
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
                format!("echo the-owner-s > '{owners}' && echo left; {read_elsewhere} true"),
            ),
            (
                &someone,
                format!(
                    "cat '{owners}' 2>/dev/null || echo read-nothing; {read_elsewhere} \
                     echo planted > '{theirs}' 2>/dev/null && cat '{theirs}' && echo wrote-own; true"
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
    let control = said[0].matches("the-person-s-elsewhere").count();
    assert_eq!(
        control,
        elsewhere.len(),
        "the person's read each: {}",
        said[0]
    );
    assert!(said[1].contains("read-nothing"), "{}", said[1]);
    assert!(!said[1].contains("the-owner-s"), "{}", said[1]);
    assert!(!said[1].contains("the-person-s"), "{}", said[1]);
    if cfg!(target_os = "linux") {
        assert!(
            said[1].contains("wrote-own"),
            "a /tmp of its own: {}",
            said[1]
        );
    }
    assert!(said[2].contains("found-nothing"), "{}", said[2]);
}
