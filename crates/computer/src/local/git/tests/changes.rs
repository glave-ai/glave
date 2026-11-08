//! What changed, read beside what a command planted: a repository it put inside the folder,
//! committed there or not, whose own config names a filter, never has it run, even with the
//! person's own `diff.submodule` set to look inside; and the person's repository, its index and
//! its objects, is left exactly as it was.

use super::*;

/// The planted repository's folder, opened in place on a computer here.
fn opened(planted: &Planted) -> (LocalComputers, WorkspaceInfo) {
    let computers = LocalComputers::new(planted.dir.join("copies"), environment(&planted.dir));
    let sources = Sources::Folder {
        path: planted.repo.to_string_lossy().into_owned(),
    };
    let opened = block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        computers.open(computer.id, &sources).await.unwrap()
    });
    (computers, opened)
}

/// The repository's index, and how many objects it holds.
fn as_it_is(planted: &Planted) -> (Vec<u8>, usize) {
    let index = std::fs::read(planted.repo.join(".git/index")).unwrap();
    let mut objects = 0;
    for folder in std::fs::read_dir(planted.repo.join(".git/objects")).unwrap() {
        if let Ok(entries) = std::fs::read_dir(folder.unwrap().path()) {
            objects += entries.count();
        }
    }
    (index, objects)
}

#[test]
fn a_filter_in_a_repository_inside_never_runs_as_changes_are_read() {
    let planted = planted();
    let (computers, opened) = opened(&planted);
    planted.repository_inside(&planted.repo.join("inner"));
    planted.holds(&planted.repo, "inner");
    planted.repository_inside(&planted.repo.join("loose"));
    plain(&planted, &planted.repo, &["status", "--porcelain"]);
    assert!(planted.take_ran(), "plain git never ran it");
    // The person's own setting, which has `diff` look inside each repository it tracks.
    let settings = planted.dir.join("home/.gitconfig");
    std::fs::write(&settings, "[diff]\n\tsubmodule = diff\n").unwrap();
    let before = as_it_is(&planted);
    let read = block_on!(computers.changes(opened.id, opened.at.as_deref()));
    let changes = read.unwrap().unwrap();
    assert!(
        !planted.ran.exists(),
        "glave's git ran a filter a repository inside names"
    );
    let paths: Vec<&str> = changes
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    assert_eq!(
        paths,
        ["inner"],
        "the one it tracks, by its commit; the other, never added, not at all"
    );
    assert_eq!(as_it_is(&planted), before, "the repository as it was");
}
