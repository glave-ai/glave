//! A git folder's `commondir`, under the real sandbox: git takes the repository's config and
//! hooks from the folder it names, so a command that wrote one would have the person's next git
//! run its code.

use super::*;

/// Points the git folder `$1` at a folder beside it.
const POINT: &str = "point() { echo ../elsewhere > \"$1/commondir\"; }; ";

/// Whether the git folder `git` names a folder beside it as where its config and hooks are.
fn points_elsewhere(git: &Path) -> bool {
    std::fs::read_to_string(git.join("commondir")).is_ok_and(|named| named.contains("elsewhere"))
}

/// Waits, up to 30 seconds, for the file `$1` to be there.
const WAIT_FOR: &str = "wait_for() { i=0; while [ ! -e \"$1\" ] && [ $i -lt 600 ]; do \
    sleep 0.05; i=$((i+1)); done; }; ";

/// Two workspaces of one repository protect its shared git folder's `commondir` at once: the
/// first's command ending leaves the second's still unable to write it (on Linux, removing the
/// stand-in under it would have taken the second's mount away).
#[test]
fn a_workspace_s_command_ending_leaves_the_shared_commondir_protected_for_another() {
    if !enforced() {
        return;
    }
    let dir = scratch("commondir-shared");
    let repo = committed_repo(&dir, "repo");
    let (computers, sandbox) = computers(&dir);
    let common = repo.join(".git");
    let c = common.display();
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        let worktree = |branch: &str| Sources::Worktree {
            path: repo.to_string_lossy().into_owned(),
            branch: branch.into(),
            owner: None,
        };
        let first = computers
            .open(computer.id, &worktree("glave/first"))
            .await
            .unwrap();
        let second = computers
            .open(computer.id, &worktree("glave/second"))
            .await
            .unwrap();
        let waits = format!(
            "{WAIT_FOR} touch '{c}/second-ready'; wait_for '{c}/first-ended'; \
             echo ../elsewhere > '{c}/commondir'; echo second-tried"
        );
        let ends = format!("{WAIT_FOR} wait_for '{c}/second-ready'; echo first-ends");
        let ((_, waited), first_said) =
            tokio::join!(run(&computers, second.id, &sandbox, &waits), async {
                let (_, said) = run(&computers, first.id, &sandbox, &ends).await;
                std::fs::write(common.join("first-ended"), "").unwrap();
                said
            });
        assert!(first_said.contains("first-ends"), "{first_said}");
        assert!(waited.contains("second-tried"), "{waited}");
        assert!(!points_elsewhere(&common), "{waited}");
    });
}

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
        assert!(!points_elsewhere(&repo.join(".git")), "{said}");
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
