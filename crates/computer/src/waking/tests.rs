//! A computer that wakes only for the calls that need it, on this machine: planning a scratch
//! folder or a project's folder, listing its commands and closing it never wake it; the first
//! call does, once, with the folder where it was planned, or says plainly that it is gone; a
//! computer woken again since opens it again; one let go of with a command running is kept for
//! it until the computer sleeps; one a call went through on is taken as awake for 30 seconds on
//! its clock.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use glave_ports::block_on;
use glave_ports::testing::{ManualClock, TempDir};

use super::*;
use crate::LocalComputers;

/// A computer of this machine with its home at `home`, woken once for each count in `wakes`
/// that `again` has moved past: each wake after the first is a computer of its own, as a Pod
/// woken again is.
fn waking(base: &Path, wakes: &Arc<AtomicUsize>, again: &Arc<AtomicUsize>) -> Home {
    let scratch = base.canonicalize().unwrap().join("scratch");
    planned_at(base, wakes, again, &scratch.to_string_lossy(), still())
}

/// A clock that stands still, so a computer a call went through on stays taken as awake.
fn still() -> Arc<dyn Clock> {
    Arc::new(ManualClock::at_second(1_791_001_000))
}

/// [`waking`], with scratch folders planned in `planned` wherever the computer makes them, and
/// how lately a call went through read on `clock`.
fn planned_at(
    base: &Path,
    wakes: &Arc<AtomicUsize>,
    again: &Arc<AtomicUsize>,
    planned: &str,
    clock: Arc<dyn Clock>,
) -> Home {
    let home = base.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let base = base.canonicalize().unwrap();
    let scratch = base.join("scratch");
    let (wakes, again) = (Arc::clone(wakes), Arc::clone(again));
    let awake: Arc<tokio::sync::Mutex<Option<(usize, Home)>>> = Arc::default();
    let root = base.join("home");
    let (seen, slept) = (Arc::clone(&awake), Arc::clone(&again));
    let peek: Peek = Arc::new(move || {
        let (seen, slept) = (Arc::clone(&seen), Arc::clone(&slept));
        Box::pin(async move {
            let seen = seen.lock().await;
            let generation = slept.load(Ordering::SeqCst);
            Ok(seen
                .as_ref()
                .filter(|(woke, _)| *woke == generation)
                .map(|(_, home)| home.clone()))
        })
    });
    let wake: Wake = Arc::new(move || {
        let (wakes, again, awake) = (Arc::clone(&wakes), Arc::clone(&again), Arc::clone(&awake));
        let (scratch, root) = (scratch.clone(), root.clone());
        Box::pin(async move {
            let mut awake = awake.lock().await;
            let generation = again.load(Ordering::SeqCst);
            if let Some((woke, home)) = awake.as_ref()
                && *woke == generation
            {
                return Ok(home.clone());
            }
            wakes.fetch_add(1, Ordering::SeqCst);
            let computers = LocalComputers::for_here(std::env::vars_os())
                .with_scratch(scratch)
                .known_as(ComputerId::from_uuid(crate::local::new_id()?));
            let (computers, info) = computers.open_here(&root).await?;
            let home = Home {
                computers: computers as Arc<dyn Computers>,
                home: info,
            };
            *awake = Some((generation, home.clone()));
            Ok(home)
        })
    });
    let home = WorkspaceInfo {
        id: WorkspaceId::from_uuid(crate::local::new_id().unwrap()),
        computer: ComputerId::from_uuid(crate::local::new_id().unwrap()),
        root: base.join("home").to_string_lossy().into_owned(),
        sources: Sources::Folder {
            path: base.join("home").to_string_lossy().into_owned(),
        },
        home: None,
        temp: None,
        at: None,
    };
    Waking::home((wake, peek), None, home, planned, clock)
}

/// The scratch folder `name`, planned on `home`.
async fn planned(home: &Home, name: &str) -> WorkspaceInfo {
    home.computers
        .open(home.home.computer, &scratch(name))
        .await
        .unwrap()
}

fn scratch(name: &str) -> Sources {
    Sources::Scratch {
        name: name.into(),
        owner: None,
    }
}

async fn run(home: &Home, workspace: WorkspaceId, script: &str) -> String {
    crate::testing::ran(home.computers.as_ref(), workspace, script).await
}

fn folder(name: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new(name).unwrap();
    let base = dir.join("people");
    std::fs::create_dir_all(&base).unwrap();
    (dir, base)
}

#[test]
fn a_scratch_folder_is_planned_asleep_and_its_first_command_wakes_the_computer_once() {
    let (_dir, base) = folder("waking-first");
    let (wakes, again) = (Arc::default(), Arc::default());
    block_on!(async {
        let home = waking(&base, &wakes, &again);
        let chat = planned(&home, "chat").await;
        let listed = home.computers.commands(chat.id).await.unwrap();
        assert_eq!(listed, Vec::new());
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            0,
            "planning and listing wake nothing"
        );

        let canonical = base.canonicalize().unwrap();
        assert_eq!(
            Path::new(&chat.root),
            canonical.join("scratch").join("chat")
        );
        let (first, second) = tokio::join!(run(&home, chat.id, "pwd"), run(&home, chat.id, "pwd"));
        assert_eq!(first.trim(), chat.root);
        assert_eq!(second.trim(), chat.root);
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            1,
            "two calls at once share one wake"
        );

        let quiet = planned(&home, "quiet").await;
        home.computers.close(quiet.id).await.unwrap();
        assert!(!Path::new(&quiet.root).exists(), "never opened, never made");
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    });
}

/// Woken again as another computer, its disk kept, however lately a call on the one before went
/// through: the server having let go of it, `TRUSTED` never stands in for it.
#[test]
fn a_computer_woken_again_opens_its_workspaces_again_however_lately_a_call_went_through() {
    let (_dir, base) = folder("waking-again");
    let (wakes, again) = (Arc::default(), Arc::<AtomicUsize>::default());
    block_on!(async {
        let home = waking(&base, &wakes, &again);
        let chat = planned(&home, "chat").await;
        run(&home, chat.id, "echo kept > kept.txt").await;
        again.fetch_add(1, Ordering::SeqCst);
        let read = home.computers.read(chat.id, "kept.txt").await.unwrap();
        assert_eq!(read, Bytes::new(b"kept\n"));
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
    });
}

/// Taken as awake for 30 seconds on its clock after a call went through on it, however long
/// they take on this machine's: past them, what changed is not asked, since asking could wake
/// it, until a call goes through again.
#[test]
fn a_computer_is_taken_as_awake_for_30_seconds_on_its_clock_after_a_call() {
    let (_dir, base) = folder("waking-trusted");
    let (wakes, again) = (Arc::default(), Arc::<AtomicUsize>::default());
    let clock = Arc::new(ManualClock::at_second(1_791_001_000));
    let scratch = base.canonicalize().unwrap().join("scratch");
    let on = Arc::clone(&clock) as Arc<dyn Clock>;
    block_on!(async {
        let home = planned_at(&base, &wakes, &again, &scratch.to_string_lossy(), on);
        let at = home.home.id;
        run(&home, at, "true").await;
        clock.advance_millis(29_999);
        let lately = home.computers.changes(at, None).await;
        assert!(
            lately.is_ok(),
            "a call went through 29.999 s ago: {lately:?}"
        );
        clock.advance_millis(30_000);
        let quiet = home.computers.changes(at, None).await;
        assert!(
            matches!(quiet, Err(ComputerError::Unsupported(_))),
            "30 s since a call went through, so it may be asleep: {quiet:?}"
        );
        run(&home, at, "true").await;
        let again = home.computers.changes(at, None).await;
        assert!(again.is_ok(), "a call went through just now: {again:?}");
        assert_eq!(
            wakes.load(Ordering::SeqCst),
            1,
            "never woken again meanwhile"
        );
    });
}

#[test]
fn a_project_s_folder_is_planned_asleep_and_opened_there_by_its_first_call() {
    let (_dir, base) = folder("waking-folder");
    let (wakes, again) = (Arc::default(), Arc::<AtomicUsize>::default());
    let project = base.canonicalize().unwrap().join("home/app");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("notes.txt"), "kept").unwrap();
    block_on!(async {
        let home = waking(&base, &wakes, &again);
        let path = project.to_string_lossy().into_owned();
        let sources = Sources::Folder { path: path.clone() };
        let opened = home
            .computers
            .open(home.home.computer, &sources)
            .await
            .unwrap();
        assert_eq!(opened.root, path);
        assert_eq!(wakes.load(Ordering::SeqCst), 0, "planning wakes nothing");
        let read = home.computers.read(opened.id, "notes.txt").await.unwrap();
        assert_eq!(read, Bytes::new(b"kept"));
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "its first call wakes it");

        // Gone from the computer, it says so in plain words at the first call that needs it.
        std::fs::remove_dir_all(&project).unwrap();
        let gone = home
            .computers
            .open(home.home.computer, &sources)
            .await
            .unwrap();
        let refused = home.computers.read(gone.id, "notes.txt").await;
        assert!(
            matches!(&refused, Err(ComputerError::NotFound(m))
                if m.starts_with(&format!("this project's folder, {path}, isn't on your computer"))),
            "{refused:?}"
        );
    });
}

#[test]
fn a_folder_not_where_it_was_planned_is_refused() {
    let (_dir, base) = folder("waking-moved");
    let (wakes, again) = (Arc::default(), Arc::default());
    block_on!(async {
        let home = planned_at(&base, &wakes, &again, "/nowhere/scratch", still());
        let chat = planned(&home, "chat").await;
        let refused = home.computers.read(chat.id, "a.txt").await;
        assert!(
            matches!(&refused, Err(ComputerError::Failed(m)) if m.contains("/nowhere/scratch/chat")),
            "{refused:?}"
        );
    });
}

#[test]
fn a_workspace_let_go_of_with_a_command_running_is_kept_for_it_until_the_computer_sleeps() {
    let (_dir, base) = folder("waking-closing");
    let (wakes, again) = (Arc::default(), Arc::<AtomicUsize>::default());
    block_on!(async {
        let home = waking(&base, &wakes, &again);
        let chat = planned(&home, "chat").await;
        let server = Command::new("sleep 30");
        let started = home.computers.start(chat.id, &server).await.unwrap();
        home.computers.close(chat.id).await.unwrap();
        let kept = home.computers.commands(chat.id).await.unwrap();
        assert_eq!(kept.len(), 1, "its command is still read there");
        home.computers.stop(chat.id, started).await.unwrap();

        // It slept, ending the command, and woke again for another workspace's call: the
        // one kept for its command is forgotten.
        again.fetch_add(1, Ordering::SeqCst);
        let other = planned(&home, "other").await;
        run(&home, other.id, "true").await;
        let gone = home.computers.commands(chat.id).await;
        assert!(matches!(gone, Err(ComputerError::NotFound(_))), "{gone:?}");
        home.computers.close(other.id).await.unwrap();
        assert_eq!(wakes.load(Ordering::SeqCst), 2, "letting go wakes nothing");
    });
}

#[test]
fn what_changed_in_a_folder_planned_asleep_is_read_since_it_was_first_opened_awake() {
    let (_dir, base) = folder("waking-changes");
    let (wakes, again) = (Arc::default(), Arc::<AtomicUsize>::default());
    let project = base.canonicalize().unwrap().join("home/app");
    std::fs::create_dir_all(&project).unwrap();
    let commit = "git -c user.name=t -c user.email=t@example.com commit -q";
    block_on!(async {
        let home = waking(&base, &wakes, &again);
        let made =
            format!("cd app && git init -q && echo a > a.txt && git add a.txt && {commit} -m a");
        run(&home, home.home.id, &made).await;
        let sources = Sources::Folder {
            path: project.to_string_lossy().into_owned(),
        };
        let opened = home
            .computers
            .open(home.home.computer, &sources)
            .await
            .unwrap();
        assert_eq!(opened.at, None, "planned, where it stands isn't known");
        let unopened = home.computers.changes(opened.id, None).await;
        assert!(
            matches!(unopened, Err(ComputerError::Unsupported(_))),
            "not open there yet, so not asked: {unopened:?}"
        );
        let committed = format!("echo c > c.txt && git add c.txt && {commit} -m c");
        run(&home, opened.id, &committed).await;
        let read = home.computers.changes(opened.id, None).await;
        let changes = read.unwrap().unwrap();
        let paths: Vec<&str> = changes
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(
            (changes.since, paths),
            (Since::Opened, vec!["c.txt"]),
            "since it was first opened awake, its commit since counted"
        );
    });
}
