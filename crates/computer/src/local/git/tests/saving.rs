//! A save: it commits everything to a branch and pushes it to the repository's `origin`, or keeps
//! the commit where there is none.

use glave_ports::computer::Save;

use super::*;

/// What a save pushes to, for the tests to look in.
const BRANCH: &str = "glave/wip/test";

#[test]
fn a_folder_saved_pushes_to_its_origin_and_without_one_keeps_the_commit() {
    let planted = planted();
    let computers = LocalComputers::new(planted.dir.join("copies"), environment(&planted.dir));
    let sources = Sources::Folder {
        path: planted.repo.to_string_lossy().into_owned(),
    };
    let save = Save {
        branch: BRANCH.into(),
        message: "work".into(),
    };
    let saved = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let opened = computers.open(computer.id, &sources).await.unwrap();
        computers.save(opened.id, &save).await.unwrap()
    });
    assert!(!saved.pushed, "no origin, so nothing to push to");
    assert_eq!(must(&planted, &["rev-parse", "HEAD"]), saved.commit);

    let origin = planted.dir.join("elsewhere.git");
    let origin = origin.to_string_lossy();
    must(&planted, &["init", "-q", "--bare", &origin]);
    must(&planted, &["remote", "add", "origin", &origin]);
    let saved = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let opened = computers.open(computer.id, &sources).await.unwrap();
        computers.save(opened.id, &save).await.unwrap()
    });
    assert!(saved.pushed);
    let at = format!("refs/heads/{BRANCH}");
    let pushed = plain(&planted, Path::new(&*origin), &["rev-parse", &at]);
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        saved.commit,
        "its push reached the origin"
    );
}
