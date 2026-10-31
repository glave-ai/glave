use std::collections::BTreeSet;
use std::process::Command as Process;
use std::sync::Arc;

use glave_ports::block_on;
use glave_ports::computer::{Computers, Environment, FileWrite, Freed, Sources};

use super::*;
use crate::LocalComputers;

fn environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect()
}

fn sh(at: &Path, script: &str) -> String {
    let out = Process::new("bash")
        .args(["-c", script])
        .current_dir(at)
        .output()
        .unwrap();
    assert!(out.status.success(), "{script}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A folder removed when it drops, a repository in it with one commit pushed to a remote,
/// and where its worktrees go.
fn repository(name: &str) -> (glave_ports::testing::TempDir, PathBuf, PathBuf) {
    let dir = glave_ports::testing::TempDir::new(&format!("worktrees-{name}")).unwrap();
    sh(
        &dir,
        "git init -q --bare --initial-branch=main origin.git && \
         git init -q --initial-branch=main app && cd app && echo hi > a.txt && git add a.txt && \
         git -c user.name=t -c user.email=t@example.com commit -q -m one && \
         git remote add origin \"$(cd .. && pwd)/origin.git\" && git push -q origin main",
    );
    let (app, worktrees) = (dir.join("app"), dir.join("worktrees"));
    (dir, app, worktrees)
}

fn worktree(repo: &Path, branch: &str, owner: Option<&str>) -> Sources {
    Sources::Worktree {
        path: repo.to_string_lossy().into_owned(),
        branch: branch.to_owned(),
        owner: owner.map(str::to_owned),
    }
}

fn file(path: &str, content: &[u8]) -> FileWrite {
    FileWrite::new(path, glave_ports::computer::Bytes::new(content))
}

fn computers(base: &Path) -> LocalComputers {
    LocalComputers::new(base.join("copies"), environment()).with_worktrees(base)
}

fn branches(repo: &Path) -> String {
    sh(
        repo,
        "git branch --list 'glave/*' --format='%(refname:short)'",
    )
}

/// Sixteen at once, so that without one change at a time two `git worktree add`s overlap and
/// one reads the other's half-made worktree.
#[test]
fn worktrees_opened_at_once_do_not_collide() {
    let (_dir, repo, base) = repository("parallel");
    let computers = Arc::new(computers(&base));
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let mut opening = tokio::task::JoinSet::new();
        for n in 0..16 {
            let computers = Arc::clone(&computers);
            let sources = worktree(&repo, &format!("glave/at-once-{n}"), None);
            opening.spawn(async move { computers.open(made.id, &sources).await });
        }
        let opened: Vec<_> = opening
            .join_all()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();
        let roots: BTreeSet<&str> = opened.iter().map(|info| info.root.as_str()).collect();
        assert_eq!(roots.len(), opened.len(), "each has a folder of its own");
        let base = std::fs::canonicalize(&base).unwrap();
        for root in roots {
            assert!(
                Path::new(root).starts_with(&base),
                "{root} is under the base"
            );
        }
        let [a, b, ..] = opened.as_slice() else {
            panic!("opened {}", opened.len())
        };
        computers
            .write(a.id, &[file("a.txt", b"changed in a")])
            .await
            .unwrap();
        let b_sees = computers.read(b.id, "a.txt").await.unwrap();
        assert_eq!(b_sees.as_slice(), b"hi\n");
        let main_sees = std::fs::read(repo.join("a.txt")).unwrap();
        assert_eq!(main_sees, b"hi\n", "the main checkout is untouched");
    });
}

#[test]
fn safe_means_nothing_uncommitted_and_every_commit_on_a_remote() {
    let (_dir, repo, base) = repository("safety");
    let computers = computers(&base);
    let env = environment();
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let clean = computers
            .open(made.id, &worktree(&repo, "glave/clean", Some("thread-1")))
            .await
            .unwrap();
        let dirty = computers
            .open(made.id, &worktree(&repo, "glave/dirty", None))
            .await
            .unwrap();
        computers
            .write(dirty.id, &[file("new.txt", b"work")])
            .await
            .unwrap();
        let ahead = computers
            .open(made.id, &worktree(&repo, "glave/ahead", None))
            .await
            .unwrap();
        let commit = "echo b > b.txt && git add b.txt && \
            git -c user.name=t -c user.email=t@example.com commit -q -m two";
        sh(Path::new(&ahead.root), commit);

        let kept = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        let seen: Vec<(Option<&str>, Condition)> = kept
            .iter()
            .map(|kept| (kept.branch.as_deref(), kept.condition))
            .collect();
        let unsafe_with = |changed, unpushed| Condition::Unsafe { changed, unpushed };
        assert_eq!(
            seen,
            [
                (Some("glave/ahead"), unsafe_with(false, 1)),
                (Some("glave/clean"), Condition::Safe),
                (Some("glave/dirty"), unsafe_with(true, 0)),
            ]
        );
        assert_eq!(kept[0].at_risk().as_deref(), Some("1 commit not pushed"));
        assert_eq!(kept[2].at_risk().as_deref(), Some("uncommitted changes"));
        assert_eq!(kept[1].owner.as_deref(), Some("thread-1"));
        assert_eq!(
            kept[1].path,
            std::fs::canonicalize(&clean.root).unwrap(),
            "listed by its folder"
        );

        // Pushed anywhere (as a squash merge leaves it, the branch on the remote) is safe.
        sh(Path::new(&ahead.root), "git push -q origin glave/ahead");
        let kept = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        assert_eq!(kept[0].condition, Condition::Safe);
    });
}

#[test]
fn a_release_removes_what_is_safe_and_holds_the_rest_keeping_every_branch() {
    let (_dir, repo, base) = repository("release");
    let computers = computers(&base);
    let env = environment();
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let clean = computers
            .open(made.id, &worktree(&repo, "glave/clean", None))
            .await
            .unwrap();
        let dirty = computers
            .open(made.id, &worktree(&repo, "glave/dirty", None))
            .await
            .unwrap();
        computers
            .write(dirty.id, &[file("new.txt", b"work")])
            .await
            .unwrap();
        let kept = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        let mut done = Vec::new();
        for kept in &kept {
            done.push(
                release(kept, &env, &Processes::unrecorded(), "idle", None)
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(
            done,
            [
                Released::Removed,
                Released::Held("uncommitted changes".into())
            ]
        );
        assert!(!Path::new(&clean.root).exists());
        assert!(Path::new(&dirty.root).join("new.txt").exists());
        let left = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].held.as_deref(), Some("idle"));
        assert_eq!(branches(&repo), "glave/clean\nglave/dirty", "branches stay");

        // Opening it again, as a resumed thread does, clears the hold.
        computers
            .open(made.id, &worktree(&repo, "glave/dirty", None))
            .await
            .unwrap();
        assert_eq!(
            list(&base, &env, &Processes::unrecorded()).await.unwrap()[0].held,
            None
        );
        remove(&left[0], &env, &Processes::unrecorded())
            .await
            .unwrap();
        assert_eq!(
            list(&base, &env, &Processes::unrecorded()).await.unwrap(),
            []
        );
        assert_eq!(branches(&repo), "glave/clean\nglave/dirty");
    });
}

#[test]
fn scratch_folders_reopen_as_left_and_go_empty_away_or_full_to_the_trash() {
    let base = glave_ports::testing::TempDir::new("scratch").unwrap();
    let computers =
        LocalComputers::new(base.join("copies"), environment()).with_scratch(base.join("scratch"));
    let env = environment();
    let scratch = |name: &str| Sources::Scratch {
        name: name.to_owned(),
        owner: Some(format!("org/project/{name}")),
    };
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let refused = computers.open(made.id, &scratch("../escape")).await;
        assert!(
            matches!(refused, Err(ComputerError::Refused(_))),
            "{refused:?}"
        );
        let full = computers.open(made.id, &scratch("full")).await.unwrap();
        computers
            .write(full.id, &[file("notes.md", b"keep me")])
            .await
            .unwrap();
        computers.open(made.id, &scratch("empty")).await.unwrap();
        let again = computers.open(made.id, &scratch("full")).await.unwrap();
        assert_eq!(again.root, full.root, "the same folder again");
        assert!(
            Path::new(&again.root).join("notes.md").exists(),
            "as it was left"
        );

        let kept = list_scratch(&base.join("scratch")).await.unwrap();
        let conditions: Vec<_> = kept.iter().map(|kept| kept.condition).collect();
        assert_eq!(
            conditions,
            [
                Condition::Scratch { empty: true },
                Condition::Scratch { empty: false }
            ]
        );
        assert_eq!(kept[0].owner.as_deref(), Some("org/project/empty"));
        assert_eq!(kept[0].at_risk(), None);
        assert_eq!(
            release(&kept[0], &env, &Processes::unrecorded(), "idle", None)
                .await
                .unwrap(),
            Released::Removed
        );
        assert_eq!(
            release(&kept[1], &env, &Processes::unrecorded(), "idle", None)
                .await
                .unwrap(),
            Released::NoTrash,
            "files are never deleted outright"
        );
        let trash = Trash::Folder(base.join("Trash"));
        let Released::Trashed(moved) = release(
            &kept[1],
            &env,
            &Processes::unrecorded(),
            "idle",
            Some(&trash),
        )
        .await
        .unwrap() else {
            panic!("trashed")
        };
        assert!(
            moved.join("notes.md").exists(),
            "everything is in the Trash"
        );
        assert_eq!(list_scratch(&base.join("scratch")).await.unwrap(), []);
    });
}

#[test]
fn a_worktree_whose_repository_is_gone_goes_to_the_trash_never_away() {
    let (_dir, repo, base) = repository("gone");
    let computers = computers(&base);
    let env = environment();
    let trash = base.with_file_name("Trash");
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let first = computers
            .open(made.id, &worktree(&repo, "glave/one", None))
            .await
            .unwrap();
        computers
            .open(made.id, &worktree(&repo, "glave/two", None))
            .await
            .unwrap();
        std::fs::remove_dir_all(&repo).unwrap();
        let kept = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        assert!(
            kept.iter()
                .all(|kept| kept.condition == Condition::RepositoryGone),
            "{kept:?}"
        );
        assert_eq!(kept[0].at_risk().as_deref(), Some("its repository is gone"));

        assert_eq!(
            release(&kept[1], &env, &Processes::unrecorded(), "idle", None)
                .await
                .unwrap(),
            Released::NoTrash
        );
        assert!(kept[1].path.exists(), "with no Trash it is kept");

        let folder = Trash::Folder(trash.join("mac"));
        let Released::Trashed(moved) = release(
            &kept[0],
            &env,
            &Processes::unrecorded(),
            "idle",
            Some(&folder),
        )
        .await
        .unwrap() else {
            panic!("trashed")
        };
        assert!(!Path::new(&first.root).exists());
        assert!(moved.join("a.txt").exists(), "everything is in the Trash");

        let desktop = Trash::Freedesktop {
            root: trash.join("xdg"),
            deleted_at: "2026-10-10T12:00:00".into(),
        };
        let Released::Trashed(moved) = release(
            &kept[1],
            &env,
            &Processes::unrecorded(),
            "idle",
            Some(&desktop),
        )
        .await
        .unwrap() else {
            panic!("trashed")
        };
        assert_eq!(moved, trash.join("xdg/files/glave-two"));
        let note = std::fs::read_to_string(trash.join("xdg/info/glave-two.trashinfo")).unwrap();
        assert!(note.starts_with("[Trash Info]\nPath=/"), "{note}");
        assert!(
            note.ends_with("DeletionDate=2026-10-10T12:00:00\n"),
            "{note}"
        );
        assert_eq!(
            list(&base, &env, &Processes::unrecorded()).await.unwrap(),
            []
        );
    });
}

#[test]
fn a_folder_outside_a_repository_is_refused_clearly() {
    let (_dir, repo, base) = repository("plain");
    let plain = repo.with_file_name("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let computers = computers(&base);
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let refused = computers
            .open(made.id, &worktree(&plain, "glave/x", None))
            .await
            .unwrap_err();
        assert!(
            matches!(&refused, ComputerError::Refused(message)
                if message.contains("not in a git repository")),
            "{refused:?}"
        );
        let without = LocalComputers::new(base.join("copies"), environment());
        let made = without.create(&Environment::host()).await.unwrap();
        let unsupported = without
            .open(made.id, &worktree(&repo, "glave/x", None))
            .await;
        assert!(matches!(unsupported, Err(ComputerError::Unsupported(_))));
    });
}

/// A record torn by a crash is read as none (the worktree is still found by its git link), and
/// the next write puts a whole one in its place; so is a record with no version.
#[test]
fn a_torn_record_is_read_as_none_and_written_whole_again() {
    let (_dir, repo, base) = repository("torn");
    let computers = computers(&base);
    let env = environment();
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let sources = worktree(&repo, "glave/torn", Some("thread-1"));
        computers.open(made.id, &sources).await.unwrap();
        let kept = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        let record = record_path(&kept[0].path);
        let whole = std::fs::read(&record).unwrap();
        let text = String::from_utf8_lossy(&whole).into_owned();
        assert!(text.contains("\"version\": 1"), "{text}");

        std::fs::write(&record, &whole[..whole.len() / 2]).unwrap();
        let torn = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        assert_eq!(torn.len(), 1, "still found");
        assert_eq!(torn[0].owner, None);
        computers.open(made.id, &sources).await.unwrap();
        assert_eq!(
            std::fs::read(&record).unwrap(),
            whole,
            "written whole again"
        );

        let unversioned = text.replace("\"version\": 1,", "");
        std::fs::write(&record, unversioned).unwrap();
        let kept = list(&base, &env, &Processes::unrecorded()).await.unwrap();
        assert_eq!(kept[0].owner, None, "read as none");
    });
}

/// A record a newer glave wrote is refused plainly, by a listing and by a write, and left as it
/// is.
#[test]
fn a_record_of_a_newer_version_is_refused_plainly() {
    let (_dir, repo, base) = repository("newer");
    let computers = computers(&base);
    let env = environment();
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let sources = worktree(&repo, "glave/newer", None);
        computers.open(made.id, &sources).await.unwrap();
        let record =
            record_path(&list(&base, &env, &Processes::unrecorded()).await.unwrap()[0].path);
        let newer = r#"{"version": 2, "repository": "/elsewhere", "branch": "b"}"#;
        std::fs::write(&record, newer).unwrap();

        let refused = list(&base, &env, &Processes::unrecorded())
            .await
            .unwrap_err();
        assert!(
            matches!(&refused, ComputerError::Refused(message)
                if message.contains("newer version of glave")),
            "{refused:?}"
        );
        let refused = computers.open(made.id, &sources).await.unwrap_err();
        assert!(matches!(refused, ComputerError::Refused(_)), "{refused:?}");
        assert_eq!(
            std::fs::read_to_string(&record).unwrap(),
            newer,
            "left as is"
        );
    });
}

#[test]
fn a_worktree_is_freed_only_when_all_it_holds_can_be_rebuilt_and_nothing_has_it_open() {
    let (_dir, repo, base) = repository("free");
    let computers = computers(&base);
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let opened = computers
            .open(made.id, &worktree(&repo, "glave/free", Some("owner")))
            .await
            .unwrap();
        let path = opened.root.clone();
        let free = async |path: &str| computers.free_worktree(made.id, path).await.unwrap();
        let kept = |freed: Freed, says: &str| match freed {
            Freed::Kept { why } => assert!(why.contains(says), "{why}"),
            Freed::Removed => panic!("kept, because {says}"),
        };
        kept(free(&path).await, "open");
        computers.close(opened.id).await.unwrap();

        let listed = computers.worktrees(made.id).await.unwrap();
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].path, path);
        assert_eq!(listed[0].owner.as_deref(), Some("owner"));

        // What a build remakes goes with it; what's uncommitted, or the person's, keeps it.
        let root = Path::new(&path);
        std::fs::write(root.join("new.txt"), "work").unwrap();
        kept(free(&path).await, "uncommitted");
        std::fs::remove_file(root.join("new.txt")).unwrap();
        std::fs::write(root.join(".gitignore"), "target\n.env\n").unwrap();
        sh(
            root,
            "git add .gitignore && git -c user.name=t -c user.email=t@example.com commit -qm i",
        );
        sh(root, "git push -q origin HEAD");
        std::fs::write(root.join(".env"), "mine").unwrap();
        kept(free(&path).await, "ignores");
        std::fs::remove_file(root.join(".env")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/built"), "x").unwrap();
        assert_eq!(free(&path).await, Freed::Removed);
        assert!(!root.exists());
        assert!(branches(&repo).contains("glave/free"), "its branch stays");
        kept(free(&path).await, "not here");
    });
}
