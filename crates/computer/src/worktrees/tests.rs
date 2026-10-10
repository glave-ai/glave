use std::process::Command as Process;

use glave_ports::{Computers, Environment, FileWrite, Sources};

use super::*;
use crate::LocalComputers;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect()
}

/// A repository with one commit, and where its worktrees go.
fn repository(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("glave-worktrees-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    let script = "git init -q --initial-branch=main && echo hi > a.txt && git add a.txt && \
        git -c user.name=t -c user.email=t@example.com commit -q -m one";
    let made = Process::new("bash")
        .args(["-c", script])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(made.success());
    (repo, dir.join("worktrees"))
}

fn worktree(repo: &Path, branch: &str) -> Sources {
    Sources::Worktree {
        path: repo.to_string_lossy().into_owned(),
        branch: branch.to_owned(),
    }
}

fn file(path: &str, content: &[u8]) -> FileWrite {
    FileWrite {
        path: path.into(),
        content: glave_ports::Bytes::new(content),
    }
}

#[test]
fn worktrees_opened_at_once_do_not_collide() {
    let (repo, base) = repository("parallel");
    let computers = LocalComputers::new(base.join("copies"), environment()).with_worktrees(&base);
    block_on(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let (a, b) = (worktree(&repo, "glave/aaaa"), worktree(&repo, "glave/bbbb"));
        let (a, b) = tokio::join!(computers.open(made.id, &a), computers.open(made.id, &b));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a.root, b.root);
        let base = std::fs::canonicalize(&base).unwrap();
        for root in [&a.root, &b.root] {
            assert!(
                Path::new(root).starts_with(&base),
                "{root} is under the base"
            );
        }
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
fn prune_keeps_a_changed_worktree_unless_confirmed() {
    let (repo, base) = repository("prune");
    let computers = LocalComputers::new(base.join("copies"), environment()).with_worktrees(&base);
    let env = environment();
    block_on(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let clean = computers
            .open(made.id, &worktree(&repo, "glave/clean"))
            .await
            .unwrap();
        let dirty = computers
            .open(made.id, &worktree(&repo, "glave/dirty"))
            .await
            .unwrap();
        computers
            .write(dirty.id, &[file("new.txt", b"work")])
            .await
            .unwrap();
        let kept = list(&base, &env).await.unwrap();
        let described: Vec<(Option<&str>, bool, bool)> = kept
            .iter()
            .map(|kept| (kept.branch.as_deref(), kept.changed, kept.merged))
            .collect();
        assert_eq!(
            described,
            [
                (Some("glave/clean"), false, true),
                (Some("glave/dirty"), true, true)
            ]
        );
        assert_eq!(kept[0].repository, std::fs::canonicalize(&repo).unwrap());

        let mut asked = Vec::new();
        let pruned = prune(kept, &env, |kept| {
            asked.push(kept.branch.clone());
            false
        })
        .await
        .unwrap();
        assert_eq!(
            asked,
            [Some("glave/dirty".to_owned())],
            "only the changed one asks"
        );
        assert!(matches!(
            &pruned[..],
            [Pruned::Removed(_), Pruned::KeptChanged(_)]
        ));
        assert!(!Path::new(&clean.root).exists());
        assert!(Path::new(&dirty.root).join("new.txt").exists());

        let left = list(&base, &env).await.unwrap();
        assert_eq!(left.len(), 1);
        let pruned = prune(left, &env, |_| true).await.unwrap();
        assert!(matches!(&pruned[..], [Pruned::Removed(_)]));
        assert_eq!(list(&base, &env).await.unwrap(), []);
        let branches = Process::new("git")
            .args(["branch", "--list", "glave/*"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&branches.stdout).trim().is_empty(),
            "merged branches go with their worktrees"
        );
    });
}

#[test]
fn a_folder_outside_a_repository_is_refused_clearly() {
    let (repo, base) = repository("plain");
    let plain = repo.with_file_name("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let computers = LocalComputers::new(base.join("copies"), environment()).with_worktrees(&base);
    block_on(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let refused = computers
            .open(made.id, &worktree(&plain, "glave/x"))
            .await
            .unwrap_err();
        assert!(
            matches!(&refused, ComputerError::Refused(message)
                if message.contains("not in a git repository")),
            "{refused:?}"
        );
        let without = LocalComputers::new(base.join("copies"), environment());
        let made = without.create(&Environment::host()).await.unwrap();
        let unsupported = without.open(made.id, &worktree(&repo, "glave/x")).await;
        assert!(matches!(unsupported, Err(ComputerError::Unsupported(_))));
    });
}
