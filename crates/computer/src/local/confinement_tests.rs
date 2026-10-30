//! The security boundary of files on this machine: paths of every shape, links of every kind,
//! a link swapped while a file is opened, hard links, and secrets kept out of commands.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use glave_ports::BoxFuture;
use glave_ports::computer::{
    Bytes, Command, ComputerId, Computers, Environment, FileWrite, Glob, Listing, Output, Search,
    Sources, WorkspaceId, refuse_protected,
};
use glave_ports::conformance::{
    Confinement, confinement_case, confinement_escapes, confinement_fixture, confinement_untouched,
};
use proptest::prelude::*;

use super::LocalComputers;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn scratch(name: &str) -> (glave_ports::testing::TempDir, PathBuf, PathBuf) {
    let dir = glave_ports::testing::TempDir::new(&format!("confine-{name}")).unwrap();
    std::fs::create_dir_all(dir.join("work")).unwrap();
    let (work, copies) = (dir.join("work"), dir.join("copies"));
    (dir, work, copies)
}

fn plain_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect()
}

struct Text(Mutex<String>);

impl Output for Text {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}

async fn output_of(computers: &LocalComputers, at: WorkspaceId, script: &str) -> String {
    let text = Text(Mutex::default());
    let never = crate::CancelSwitch::default();
    computers
        .run(at, &Command::new(script), &text, &never)
        .await
        .unwrap();
    std::mem::take(&mut *text.0.lock().unwrap())
}

/// A workspace on a folder, and the fixture built inside it.
async fn fixture(
    name: &str,
) -> (
    LocalComputers,
    PathBuf,
    ComputerId,
    WorkspaceId,
    Confinement,
    glave_ports::testing::TempDir,
) {
    let (dir, work, copies) = scratch(name);
    let computers = LocalComputers::new(&copies, plain_environment());
    let made = computers.create(&Environment::host()).await.unwrap();
    let path = work.to_string_lossy().into_owned();
    let here = computers
        .open(made.id, &Sources::Folder { path: path.clone() })
        .await
        .unwrap();
    let confinement = confinement_fixture(&computers, made.id, here.id, &path)
        .await
        .unwrap();
    (computers, work, made.id, here.id, confinement, dir)
}

/// A path as models and attackers spell them: pieces that make trouble, and any text.
fn spelling() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => prop::sample::select(vec![
            "..", ".", "", "/", "~", "out_dir", "out_file", "up", "grand", "self", "in_link",
            "l1", "l2", "a", "b", "dangling", "rootlink", "hard.txt", "file.txt", "sub",
            "deep_out", "new", "sentinel.txt", "outside", "back\\slash", "..\\..", ".git",
            ".glave", "hooklink", "x/",
        ])
        .prop_map(str::to_owned),
        1 => "\\PC{0,12}",
        1 => "[a-z]{200,300}",
    ];
    (prop::collection::vec(piece, 1..7), any::<bool>()).prop_map(|(pieces, trailing)| {
        format!("{}{}", pieces.join("/"), if trailing { "/" } else { "" })
    })
}

#[test]
fn no_spelling_of_a_path_gets_outside_the_workspace() {
    let rt = runtime();
    let (computers, _work, _, _, confinement, _dir) = rt.block_on(fixture("property"));
    let config = ProptestConfig {
        cases: 48,
        failure_persistence: None,
        ..ProptestConfig::default()
    };
    proptest!(config, |(path in spelling())| {
        let verdict = rt.block_on(confinement_case(&computers, &confinement, &path));
        prop_assert_eq!(verdict, Ok(()));
    });
}

#[test]
fn links_planted_by_commands_and_hard_links_cannot_reach_outside() {
    let rt = runtime();
    let (computers, _work, _, _, confinement, _dir) = rt.block_on(fixture("escapes"));
    assert_eq!(
        rt.block_on(confinement_escapes(&computers, &confinement)),
        Ok(())
    );
}

/// Runs `round` again and again while a thread swaps the folder `inside/in_dir` for a link to
/// each of `targets` in turn and back, the way a command running beside the file tools could,
/// for at least three seconds and a hundred swaps. The real folder is back when it returns.
///
/// How the two interleave is the scheduler's: a check that needs some round to land in one
/// state of the folder fails when the machine is busy though confinement held (#1028). What a
/// race test asserts holds whatever the interleaving; what still works is checked after.
fn race(inside: &std::path::Path, targets: &'static [&'static str], mut round: impl FnMut()) {
    let (swaps, stop) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let flipper = {
        let (inside, swaps, stop) = (inside.to_owned(), Arc::clone(&swaps), Arc::clone(&stop));
        std::thread::spawn(move || flip(&inside, targets, &swaps, &stop))
    };
    let started = std::time::Instant::now();
    while (started.elapsed().as_millis() < 3_000 || swaps.load(Ordering::Relaxed) < 100)
        && started.elapsed() < glave_ports::testing::HUNG
    {
        round();
    }
    stop.store(true, Ordering::Relaxed);
    flipper.join().unwrap();
    let swapped = swaps.load(Ordering::Relaxed);
    assert!(swapped >= 100, "the race never really ran: {swapped} swaps");
}

/// Swaps the folder `in_dir` for a link to each of `targets` in turn and back until told to
/// stop, counting the swaps.
fn flip(inside: &std::path::Path, targets: &[&str], swaps: &AtomicUsize, stop: &AtomicBool) {
    let (folder, parked) = (inside.join("in_dir"), inside.join("in_dir.parked"));
    while !stop.load(Ordering::Relaxed) {
        if std::fs::rename(&folder, &parked).is_ok() {
            let made = swaps.load(Ordering::Relaxed);
            let target = targets.get(made % targets.len().max(1)).unwrap();
            if std::os::unix::fs::symlink(target, &folder).is_ok() {
                swaps.fetch_add(1, Ordering::Relaxed);
            }
            let _ = std::fs::remove_file(&folder);
            // A write may have made the folder anew while it was gone; the old one goes back.
            while std::fs::rename(&parked, &folder).is_err() {
                let _ = std::fs::remove_dir_all(&folder);
            }
        }
    }
}

/// Every file operation once on the raced folder: what it showed from outside, if anything.
fn leaks(computers: &LocalComputers, at: WorkspaceId, rt: &tokio::runtime::Runtime) -> Vec<String> {
    let mut seen = Vec::new();
    for path in ["in_dir/sentinel.txt", "in_dir/fresh/new.txt"] {
        let write = FileWrite::new(path, Bytes::new(b"PWNED"));
        let _ = rt.block_on(computers.write(at, &[write]));
    }
    if let Ok(bytes) = rt.block_on(computers.read(at, "in_dir/sentinel.txt")) {
        seen.push(String::from_utf8_lossy(bytes.as_slice()).into_owned());
    }
    if let Ok(stat) = rt.block_on(computers.stat(at, "in_dir/linked.txt"))
        && stat.kind.is_some()
    {
        seen.push(format!("stat {stat:?}"));
    }
    let glob = Glob {
        pattern: "*".into(),
        path: ".".into(),
        limit: 1_000,
    };
    if let Ok(globbed) = rt.block_on(computers.glob(at, &glob)) {
        seen.extend(globbed.files);
    }
    let search = Search {
        pattern: "SENTINEL".into(),
        path: ".".into(),
        glob: None,
        ignore_case: false,
        limit: 100,
        line_chars: 200,
    };
    if let Ok(searched) = rt.block_on(computers.search(at, &search)) {
        seen.extend(searched.matches.into_iter().map(|found| found.text));
    }
    for skip_ignored in [false, true] {
        let listing = Listing {
            recursive: true,
            skip_ignored,
            ..Listing::of(".")
        };
        if let Ok(listed) = rt.block_on(computers.list(at, &listing)) {
            seen.extend(listed.entries.into_iter().map(|entry| entry.path));
        }
    }
    seen.retain(|text| text.contains("SENTINEL") || text.contains("linked.txt"));
    seen
}

/// Every file operation, raced by a command swapping a folder for a link to outside: nothing
/// outside is read, named, written or created, and writes inside still work.
#[test]
fn a_link_swapped_while_files_are_worked_on_never_reaches_outside() {
    let rt = runtime();
    let (computers, work, _, _, confinement, _dir) = rt.block_on(fixture("race"));
    let inside = work.join("confine/inside");
    std::fs::create_dir_all(inside.join("in_dir")).unwrap();
    std::fs::write(inside.join("in_dir/sentinel.txt"), "inside").unwrap();
    let outside = work.join("confine/outside");
    let workspace = confinement_workspace(&computers, &work, &rt);
    let mut leaked = Vec::new();
    race(&inside, &["../outside"], || {
        leaked.extend(leaks(&computers, workspace, &rt));
    });
    drop(confinement);
    assert!(leaked.is_empty(), "seen from outside: {leaked:?}");
    let mut names: Vec<String> = std::fs::read_dir(&outside)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["linked.txt", "sentinel.txt"],
        "nothing made outside"
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("sentinel.txt")).unwrap(),
        "SENTINEL"
    );
    let write = FileWrite::new("in_dir/sentinel.txt", Bytes::new(b"after"));
    rt.block_on(computers.write(workspace, &[write])).unwrap();
    let read = rt.block_on(computers.read(workspace, "in_dir/sentinel.txt"));
    assert_eq!(
        read.unwrap().as_slice(),
        b"after",
        "writes inside still work"
    );
    assert_eq!(
        std::fs::read_to_string(inside.join("in_dir/sentinel.txt")).unwrap(),
        "after",
        "and land in the real folder"
    );
}

/// Protected writes, raced by a command swapping a folder for a link into `.git/hooks` or
/// `.glave` and back: the computer checks the path it actually opens, so nothing protected is
/// ever written. Afterwards, a protected write into the real folder lands, and through a link
/// is refused as the path it would land on.
#[test]
fn a_link_swapped_into_protected_paths_never_lets_a_protected_write_through() {
    let rt = runtime();
    let (_dir, work, copies) = scratch("protected-race");
    std::fs::create_dir_all(work.join(".git/hooks")).unwrap();
    std::fs::create_dir_all(work.join(".glave")).unwrap();
    std::fs::write(work.join(".glave/settings.json"), "{}\n").unwrap();
    std::fs::create_dir_all(work.join("in_dir")).unwrap();
    let computers = LocalComputers::new(&copies, plain_environment());
    let here = rt.block_on(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap()
    });
    let protected = |path: &str| FileWrite {
        protect: true,
        ..FileWrite::new(path, Bytes::new(b"PWNED"))
    };
    race(&work, &[".git/hooks", ".glave"], || {
        for path in [
            "in_dir/pre-commit",
            "in_dir/settings.json",
            "in_dir/fresh/hook",
        ] {
            let _ = rt.block_on(computers.write(here.id, &[protected(path)]));
        }
    });
    let hooks: Vec<_> = std::fs::read_dir(work.join(".git/hooks"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(hooks.is_empty(), "a hook was written: {hooks:?}");
    let settings: Vec<_> = std::fs::read_dir(work.join(".glave"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(settings, ["settings.json"], "nothing new in .glave");
    assert_eq!(
        std::fs::read_to_string(work.join(".glave/settings.json")).unwrap(),
        "{}\n"
    );
    let write = || rt.block_on(computers.write(here.id, &[protected("in_dir/pre-commit")]));
    write().unwrap();
    let landed = std::fs::read_to_string(work.join("in_dir/pre-commit")).unwrap();
    assert_eq!(
        landed, "PWNED",
        "a protected write into the real folder lands"
    );
    std::fs::remove_dir_all(work.join("in_dir")).unwrap();
    for target in [".git/hooks", ".glave"] {
        std::os::unix::fs::symlink(target, work.join("in_dir")).unwrap();
        let lands = format!("{target}/pre-commit");
        assert_eq!(write(), Err(refuse_protected(&lands)), "through {target}");
        std::fs::remove_file(work.join("in_dir")).unwrap();
    }
}

/// The fixture's inside folder opened as a workspace of its own.
fn confinement_workspace(
    computers: &LocalComputers,
    work: &std::path::Path,
    rt: &tokio::runtime::Runtime,
) -> WorkspaceId {
    let made = rt.block_on(computers.create(&Environment::host())).unwrap();
    let path = work.join("confine/inside").to_string_lossy().into_owned();
    rt.block_on(computers.open(made.id, &Sources::Folder { path }))
        .unwrap()
        .id
}

/// The check that the outside is untouched sees a change, so it cannot pass vacuously.
#[test]
fn the_outside_check_sees_any_change() {
    let rt = runtime();
    let (computers, work, _, _, confinement, _dir) = rt.block_on(fixture("check"));
    let check = || rt.block_on(confinement_untouched(&computers, &confinement));
    assert_eq!(check(), Ok(()));
    let sentinel = work.join("confine/outside/sentinel.txt");
    std::fs::write(&sentinel, "SENTINEX").unwrap();
    assert!(check().is_err(), "a changed sentinel is seen");
    std::fs::write(&sentinel, "SENTINEL").unwrap();
    std::fs::write(work.join("confine/outside/new.txt"), "").unwrap();
    assert!(check().is_err(), "a file made outside is seen");
    std::fs::remove_file(work.join("confine/outside/new.txt")).unwrap();
    std::fs::write(work.join("confine/stray.txt"), "").unwrap();
    assert!(check().is_err(), "a file made beside the workspace is seen");
    std::fs::remove_file(work.join("confine/stray.txt")).unwrap();
    assert_eq!(check(), Ok(()));
}

/// Editing a file hard-linked from elsewhere (a package store does this) works, and the other
/// name keeps the old content.
#[test]
fn a_hard_linked_file_can_be_edited_and_the_other_name_is_unchanged() {
    let rt = runtime();
    let (_dir, work, copies) = scratch("hardlink");
    let store = work.with_file_name("store");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("index.js"), "store\n").unwrap();
    std::fs::create_dir_all(work.join("node_modules/pkg")).unwrap();
    std::fs::hard_link(
        store.join("index.js"),
        work.join("node_modules/pkg/index.js"),
    )
    .unwrap();
    let computers = LocalComputers::new(&copies, plain_environment());
    let here = rt.block_on(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap()
    });
    let edit = FileWrite::new("node_modules/pkg/index.js", Bytes::new(b"edited\n"));
    rt.block_on(computers.write(here.id, &[edit])).unwrap();
    assert_eq!(
        std::fs::read_to_string(work.join("node_modules/pkg/index.js")).unwrap(),
        "edited\n"
    );
    assert_eq!(
        std::fs::read_to_string(store.join("index.js")).unwrap(),
        "store\n"
    );
}

#[test]
fn commands_never_see_secret_variables() {
    let hidden = [
        "OPENAI_API_KEY",
        "glave_api_key",
        "Github_Token",
        "DB_PASSWORD",
        "my_passwd",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "aws_Secret_Thing",
        "SOME_KEY_FILE",
    ];
    let mut environment = plain_environment();
    let secret = glave_ports::testing::any_secret();
    for name in hidden {
        environment.push((name.into(), secret.clone().into()));
    }
    environment.push(("HARMLESS_SETTING".into(), "visible".into()));
    let (_dir, work, copies) = scratch("secrets");
    let computers = LocalComputers::new(&copies, environment);
    let rt = runtime();
    let seen = rt.block_on(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        let here = computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        output_of(&computers, here.id, "env").await
    });
    assert!(seen.contains("HARMLESS_SETTING=visible"), "{seen}");
    assert!(
        !seen.contains(&secret),
        "a secret reached a command: {seen}"
    );
    for name in hidden {
        assert!(!seen.contains(name), "{name} reached a command");
    }
}
