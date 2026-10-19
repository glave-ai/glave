use std::path::PathBuf;
use std::process::Command as Process;

use glave_computer::LocalComputers;
use glave_event::{
    Body, ComputerKind, ComputerRef, EventId, Header, Opened, Seq, WorkspaceBound, decode, encode,
};
use glave_ports::block_on;
use glave_ports::computer::{Computers, Environment, Sources};
use glave_ports::testing::CountingEntropy;
use glave_ports::{InMemoryLog, OrgId, SpaceId};

use super::*;
use crate::worktrees::{FOLDER, SWEPT, check_if_due};

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

const COMMIT: &str = "echo b > b.txt && git add b.txt && \
    git -c user.name=t -c user.email=t@example.com commit -q -m two";

/// A home folder, and a repository in it with one commit pushed to a remote.
fn place(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("glave-cleanup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).unwrap();
    sh(
        &dir,
        "git init -q --bare --initial-branch=main origin.git && \
         git init -q --initial-branch=main app && cd app && echo hi > a.txt && git add a.txt && \
         git -c user.name=t -c user.email=t@example.com commit -q -m one && \
         git remote add origin \"$(cd .. && pwd)/origin.git\" && git push -q origin main",
    );
    (dir.join("home"), dir.join("app"))
}

fn scope() -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        space: SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
    }
}

fn day(n: i64) -> Timestamp {
    Timestamp::from_second(1_791_000_000 + n * 86_400).unwrap()
}

/// A thread whose one event happened at `at`.
async fn thread(log: &InMemoryLog, n: u8, at: Timestamp) -> ThreadId {
    let id = ThreadId::new(at, [n; 10]).unwrap();
    let header = Header {
        id: EventId::new(at, [n; 10]).unwrap(),
        thread: id,
        seq: Seq::FIRST,
        at: at.into(),
    };
    let bound = WorkspaceBound {
        workspace: uuid::Uuid::from_u128(9),
        computer: ComputerRef {
            id: uuid::Uuid::from_u128(8),
            kind: ComputerKind::Local,
        },
        opened: Opened::Folder { path: "/".into() },
        root: "/".into(),
    };
    log.append(&scope(), &[encode(header, &bound).unwrap()])
        .await
        .unwrap();
    id
}

/// A worktree of `repo` on `branch` for `owner`.
async fn worktree(home: &Path, repo: &Path, branch: &str, owner: Option<ThreadId>) -> PathBuf {
    let computers =
        LocalComputers::new(home.join("copies"), environment()).with_worktrees(home.join(FOLDER));
    let made = computers.create(&Environment::host()).await.unwrap();
    let sources = Sources::Worktree {
        path: repo.to_string_lossy().into_owned(),
        branch: branch.into(),
        owner: owner.map(|thread| owner_label(scope(), thread)),
    };
    PathBuf::from(computers.open(made.id, &sources).await.unwrap().root)
}

async fn kept(home: &Path) -> Vec<Kept> {
    worktrees::list(&home.join(FOLDER), &environment())
        .await
        .unwrap()
}

/// The `workspace.removed` events of `thread`.
async fn removals(log: &InMemoryLog, thread: ThreadId) -> Vec<WorkspaceRemoved> {
    let events = log.read(&scope(), thread, None, 100).await.unwrap();
    events
        .into_iter()
        .filter_map(|raw| match decode(raw).unwrap().body {
            Body::WorkspaceRemoved(removed) => Some(removed),
            _ => None,
        })
        .collect()
}

fn branches(repo: &Path) -> String {
    sh(
        repo,
        "git branch --list 'glave/*' --format='%(refname:short)'",
    )
}

#[test]
fn idle_threads_lose_their_worktrees_only_when_nothing_is_lost() {
    let (home, repo) = place("idle");
    let log = InMemoryLog::new();
    let env = environment();
    block_on!(async {
        let quiet = thread(&log, 1, day(0)).await;
        let busy = thread(&log, 2, day(0)).await;
        let recent = thread(&log, 3, day(29)).await;
        let quiet_root = worktree(&home, &repo, "glave/quiet", Some(quiet)).await;
        let busy_root = worktree(&home, &repo, "glave/busy", Some(busy)).await;
        let recent_root = worktree(&home, &repo, "glave/recent", Some(recent)).await;
        sh(&busy_root, COMMIT);
        let cleanup = Cleanup {
            log: &log,
            environment: &env,
            trash: None,
            now: day(31),
            entropy: &CountingEntropy::starting_at(100),
        };
        let lines = cleanup.idle(&kept(&home).await).await;
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("1 commit not pushed"), "{lines:?}");
        assert!(lines[1].starts_with("removed"), "{lines:?}");
        assert!(!quiet_root.exists());
        assert!(busy_root.exists() && recent_root.exists());
        let removed = removals(&log, quiet).await;
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].reason, RemovedReason::Idle);
        assert_eq!(removed[0].branch.as_deref(), Some("glave/quiet"));
        assert_eq!(removals(&log, busy).await, []);
        let held: Vec<Option<String>> = kept(&home)
            .await
            .into_iter()
            .map(|kept| kept.held)
            .collect();
        assert_eq!(held, [Some("idle".to_owned()), None]);
        assert_eq!(branches(&repo), "glave/busy\nglave/quiet\nglave/recent");

        // Held once, it is not reported again; pushed, it goes.
        assert_eq!(cleanup.idle(&kept(&home).await).await, Vec::<String>::new());
        sh(&busy_root, "git push -q origin glave/busy");
        let lines = cleanup.idle(&kept(&home).await).await;
        assert!(lines[0].starts_with("removed"), "{lines:?}");
        assert!(!busy_root.exists());
    });
}

#[test]
fn the_check_runs_at_most_once_a_day() {
    let (home, repo) = place("daily");
    let log = InMemoryLog::new();
    let env = environment();
    block_on!(async {
        let quiet = thread(&log, 1, day(0)).await;
        worktree(&home, &repo, "glave/quiet", Some(quiet)).await;
        let entropy = CountingEntropy::starting_at(100);
        let at = |now| Cleanup {
            log: &log,
            environment: &env,
            trash: None,
            now,
            entropy: &entropy,
        };
        // Day 10: checked, nothing idle yet.
        assert_eq!(
            check_if_due(&home, &at(day(10))).await,
            Vec::<String>::new()
        );
        // Day 31, but a check ran less than a day ago (the stamp says day 30.5).
        std::fs::write(home.join(SWEPT), day(30).to_string()).unwrap();
        let soon = day(30) + SignedDuration::from_hours(12);
        assert_eq!(check_if_due(&home, &at(soon)).await, Vec::<String>::new());
        assert_eq!(kept(&home).await.len(), 1);
        let lines = check_if_due(&home, &at(day(31))).await;
        assert!(lines[0].starts_with("removed"), "{lines:?}");
        let stamp = std::fs::read_to_string(home.join(SWEPT)).unwrap();
        assert_eq!(stamp, day(31).to_string());
    });
}

#[test]
fn a_torn_check_stamp_is_checked_again_and_written_whole() {
    let (home, _repo) = place("torn-stamp");
    std::fs::create_dir_all(home.join(FOLDER)).unwrap();
    let log = InMemoryLog::new();
    let env = environment();
    let entropy = CountingEntropy::starting_at(100);
    let cleanup = Cleanup {
        log: &log,
        environment: &env,
        trash: None,
        now: day(10),
        entropy: &entropy,
    };
    // Half of a stamp, as a crash while writing it would leave.
    let whole = day(9).to_string();
    std::fs::write(home.join(SWEPT), whole.get(..10).unwrap()).unwrap();
    block_on!(async {
        assert_eq!(check_if_due(&home, &cleanup).await, Vec::<String>::new());
    });
    let stamp = std::fs::read_to_string(home.join(SWEPT)).unwrap();
    assert_eq!(
        stamp,
        day(10).to_string(),
        "the check ran and wrote its stamp anew"
    );
}

#[test]
fn a_gone_repositorys_worktree_goes_to_the_trash_and_is_recorded() {
    let (home, repo) = place("gone");
    let log = InMemoryLog::new();
    let env = environment();
    block_on!(async {
        let owner = thread(&log, 1, day(30)).await;
        let root = worktree(&home, &repo, "glave/orphan", Some(owner)).await;
        std::fs::remove_dir_all(&repo).unwrap();
        let entropy = CountingEntropy::starting_at(100);
        let none = Cleanup {
            log: &log,
            environment: &env,
            trash: None,
            now: day(31),
            entropy: &entropy,
        };
        let lines = none.idle(&kept(&home).await).await;
        assert!(lines[0].contains("no Trash"), "{lines:?}");
        assert!(root.exists());

        let trash = Trash::Folder(home.join(".Trash"));
        let to_trash = Cleanup {
            trash: Some(&trash),
            ..none
        };
        let lines = to_trash.idle(&kept(&home).await).await;
        assert!(lines[0].contains("to the Trash"), "{lines:?}");
        assert!(!root.exists());
        assert!(home.join(".Trash/glave-orphan/a.txt").exists());
        let removed = removals(&log, owner).await;
        assert_eq!(removed[0].reason, RemovedReason::RepositoryGone);
        assert!(
            removed[0]
                .trash
                .as_deref()
                .is_some_and(|to| to.ends_with("glave-orphan"))
        );
    });
}

#[test]
fn prune_removes_the_safe_explains_the_rest_and_forces_only_by_name() {
    let (home, repo) = place("prune");
    let log = InMemoryLog::new();
    let env = environment();
    block_on!(async {
        let owner = thread(&log, 1, day(30)).await;
        let safe = worktree(&home, &repo, "glave/safe", Some(owner)).await;
        let ahead = worktree(&home, &repo, "glave/ahead", None).await;
        let dirty = worktree(&home, &repo, "glave/dirty", None).await;
        sh(&ahead, COMMIT);
        std::fs::write(dirty.join("new.txt"), "work").unwrap();
        let cleanup = Cleanup {
            log: &log,
            environment: &env,
            trash: None,
            now: day(31),
            entropy: &CountingEntropy::starting_at(100),
        };
        let lines = cleanup.prune(&kept(&home).await, &[]).await;
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(
            lines[0].contains("1 commit not pushed")
                && lines[0].contains("prune --force glave-ahead"),
            "{lines:?}"
        );
        assert!(lines[1].contains("uncommitted changes"), "{lines:?}");
        assert!(lines[2].starts_with("removed"), "{lines:?}");
        assert!(!safe.exists() && ahead.exists() && dirty.exists());
        assert_eq!(removals(&log, owner).await[0].reason, RemovedReason::Pruned);

        let lines = cleanup
            .prune(&kept(&home).await, &["glave-ahead".to_owned()])
            .await;
        assert!(lines[0].contains("forced"), "{lines:?}");
        assert!(!ahead.exists() && dirty.exists());
        assert_eq!(branches(&repo), "glave/ahead\nglave/dirty\nglave/safe");
    });
}

#[test]
fn archiving_a_thread_releases_only_its_worktrees() {
    let (home, repo) = place("archive");
    let log = InMemoryLog::new();
    let env = environment();
    block_on!(async {
        let archived = thread(&log, 1, day(30)).await;
        let held = thread(&log, 2, day(30)).await;
        let other = thread(&log, 3, day(30)).await;
        let gone = worktree(&home, &repo, "glave/archived", Some(archived)).await;
        let kept_dirty = worktree(&home, &repo, "glave/held", Some(held)).await;
        let untouched = worktree(&home, &repo, "glave/other", Some(other)).await;
        std::fs::write(kept_dirty.join("new.txt"), "work").unwrap();
        let cleanup = Cleanup {
            log: &log,
            environment: &env,
            trash: None,
            now: day(31),
            entropy: &CountingEntropy::starting_at(100),
        };
        let all = kept(&home).await;
        cleanup.archived(&all, scope(), archived).await;
        let lines = cleanup.archived(&all, scope(), held).await;
        assert!(
            lines[0].contains("(archived): uncommitted changes"),
            "{lines:?}"
        );
        assert!(!gone.exists() && kept_dirty.exists() && untouched.exists());
        assert_eq!(
            removals(&log, archived).await[0].reason,
            RemovedReason::Archived
        );
        let holds: Vec<Option<String>> = kept(&home)
            .await
            .into_iter()
            .map(|kept| kept.held)
            .collect();
        assert_eq!(holds, [Some("archived".to_owned()), None]);
    });
}
