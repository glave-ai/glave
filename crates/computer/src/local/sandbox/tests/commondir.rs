//! A git folder's `commondir`, under the real sandbox: git takes the repository's config and
//! hooks from the folder it names, so a command that wrote one would have the person's next git
//! run its code.

use super::*;

/// Points the git folder `$1` at a folder beside it.
const POINT: &str = "point() { echo ../elsewhere > \"$1/commondir\"; }; ";

/// No sandboxed command makes a repository's git folder name another, nor changes the one a
/// worktree's names; git still works in both.
#[test]
fn no_sandboxed_command_points_git_at_another_folder() {
    if !enforced() {
        return;
    }
    let dir = scratch("commondir");
    let repo = committed_repo(&dir, "repo");
    let (computers, sandbox) = computers(&dir);
    let own = repo.join(".git/worktrees/glave-mine");
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let path = repo.to_string_lossy().into_owned();
        let here = computers
            .open(computer.id, &Sources::Folder { path })
            .await
            .unwrap();
        let (_, said) = run(
            &computers,
            here.id,
            &sandbox,
            &format!("{POINT} point .git; echo done"),
        )
        .await;
        assert!(said.contains("done"), "{said}");
        assert!(!repo.join(".git/commondir").exists(), "{said}");
        // git reads the repository as it did (on Linux, through the stand-in), and the
        // worktree below, made from it, still commits.
        let (ending, said) = run(
            &computers,
            here.id,
            &sandbox,
            "git status -s && echo git-ok",
        )
        .await;
        assert_eq!(ending, Ending::Exited { code: 0 }, "{said}");
        assert!(said.contains("git-ok"), "{said}");

        let worktree = Sources::Worktree {
            path: repo.to_string_lossy().into_owned(),
            branch: "glave/mine".into(),
            owner: None,
        };
        let mine = computers.open(computer.id, &worktree).await.unwrap();
        let named = std::fs::read_to_string(own.join("commondir")).unwrap();
        let script = format!("{POINT} point '{}'; echo done", own.display());
        let (_, said) = run(&computers, mine.id, &sandbox, &script).await;
        assert!(said.contains("done"), "{said}");
        let now = std::fs::read_to_string(own.join("commondir")).unwrap();
        assert_eq!(now, named, "{said}");
        git_works(&computers, mine.id, &sandbox).await;
    });
    assert!(!repo.join(".git/commondir").exists(), "no stand-in is left");
    // Control: unsandboxed, the same command points it elsewhere, so the checks above can fail.
    let open = committed_repo(&dir, "open");
    let made = Process::new("bash")
        .args(["-c", &format!("{POINT} point .git")])
        .current_dir(&open)
        .status()
        .unwrap();
    assert!(made.success());
    assert!(open.join(".git/commondir").exists());
}
