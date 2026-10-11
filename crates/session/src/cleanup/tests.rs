use std::path::PathBuf;
use std::process::Command as Process;
use std::sync::atomic::{AtomicU64, Ordering};

use glave_computer::LocalComputers;
use glave_event::{
    Body, ComputerKind, ComputerRef, EventId, Header, Opened, Seq, WorkspaceBound, decode, encode,
};
use glave_ports::{Computers, Environment, MemoryLog, OrgId, Sources, SpaceId};

use super::*;

struct Counting(AtomicU64);

impl Entropy for Counting {
    fn bytes(&self) -> [u8; 10] {
        let n = self.0.fetch_add(1, Ordering::SeqCst).to_be_bytes();
        [n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7], 0, 0]
    }
}

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
async fn thread(log: &MemoryLog, n: u8, at: Timestamp) -> ThreadId {
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
async fn removals(log: &MemoryLog, thread: ThreadId) -> Vec<WorkspaceRemoved> {
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
    let log = MemoryLog::new();
    let env = environment();
    block_on(async {
        let quiet = thread(&log, 1, day(0)).await;
        let busy = thread(&log, 2, day(0)).await;
        let recent = thread(&log, 3, day(29)).await;
        let quiet_root = worktree(&home, &repo, "glave/quiet", Some(quiet)).await;
        let busy_root = worktree(&home, &repo, "glave/busy", Some(busy)).await;
        let recent_root = worktree(&home, &repo, "glave/recent", Some(recent)).await;
        sh(&busy_root, COMMIT);
        let cleanup = Cleanup {
            ledger: &InLog(&log),
            environment: &env,
            trash: None,
            now: day(31),
            entropy: &Counting(AtomicU64::new(100)),
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
    let log = MemoryLog::new();
    let env = environment();
    block_on(async {
        let quiet = thread(&log, 1, day(0)).await;
        worktree(&home, &repo, "glave/quiet", Some(quiet)).await;
        let entropy = Counting(AtomicU64::new(100));
        let ledger = InLog(&log);
        let at = |now| Cleanup {
            ledger: &ledger,
            environment: &env,
            trash: None,
            now,
            entropy: &entropy,
        };
        // Day 10: due, and the stamp says so; nothing is idle yet.
        assert!(is_due(&home, day(10)).await.unwrap());
        assert_eq!(
            at(day(10)).idle(&kept(&home).await).await,
            Vec::<String>::new()
        );
        // Day 31, but a check ran less than a day ago (the stamp says day 30.5).
        std::fs::write(home.join(SWEPT), day(30).to_string()).unwrap();
        let soon = day(30) + SignedDuration::from_hours(12);
        assert!(!is_due(&home, soon).await.unwrap());
        assert_eq!(kept(&home).await.len(), 1);
        assert!(is_due(&home, day(31)).await.unwrap());
        let lines = at(day(31)).idle(&kept(&home).await).await;
        assert!(lines[0].starts_with("removed"), "{lines:?}");
        let stamp = std::fs::read_to_string(home.join(SWEPT)).unwrap();
        assert_eq!(stamp, day(31).to_string());
    });
}

#[test]
fn a_gone_repositorys_worktree_goes_to_the_trash_and_is_recorded() {
    let (home, repo) = place("gone");
    let log = MemoryLog::new();
    let env = environment();
    block_on(async {
        let owner = thread(&log, 1, day(30)).await;
        let root = worktree(&home, &repo, "glave/orphan", Some(owner)).await;
        std::fs::remove_dir_all(&repo).unwrap();
        let entropy = Counting(AtomicU64::new(100));
        let none = Cleanup {
            ledger: &InLog(&log),
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
    let log = MemoryLog::new();
    let env = environment();
    block_on(async {
        let owner = thread(&log, 1, day(30)).await;
        let safe = worktree(&home, &repo, "glave/safe", Some(owner)).await;
        let ahead = worktree(&home, &repo, "glave/ahead", None).await;
        let dirty = worktree(&home, &repo, "glave/dirty", None).await;
        sh(&ahead, COMMIT);
        std::fs::write(dirty.join("new.txt"), "work").unwrap();
        let cleanup = Cleanup {
            ledger: &InLog(&log),
            environment: &env,
            trash: None,
            now: day(31),
            entropy: &Counting(AtomicU64::new(100)),
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
    let log = MemoryLog::new();
    let env = environment();
    block_on(async {
        let archived = thread(&log, 1, day(30)).await;
        let held = thread(&log, 2, day(30)).await;
        let other = thread(&log, 3, day(30)).await;
        let gone = worktree(&home, &repo, "glave/archived", Some(archived)).await;
        let kept_dirty = worktree(&home, &repo, "glave/held", Some(held)).await;
        let untouched = worktree(&home, &repo, "glave/other", Some(other)).await;
        std::fs::write(kept_dirty.join("new.txt"), "work").unwrap();
        let cleanup = Cleanup {
            ledger: &InLog(&log),
            environment: &env,
            trash: None,
            now: day(31),
            entropy: &Counting(AtomicU64::new(100)),
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
