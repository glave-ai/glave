//! The security boundary of files on this machine: paths of every shape, links of every kind,
//! a link swapped while a file is opened, hard links, and secrets kept out of commands.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use glave_ports::conformance::{
    Confinement, confinement_case, confinement_escapes, confinement_fixture,
};
use glave_ports::{
    BoxFuture, Bytes, Command, ComputerId, Computers, Environment, FileWrite, Output, Sources,
    WorkspaceId,
};
use proptest::prelude::*;

use super::LocalComputers;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn scratch(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("glave-confine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    (dir.join("work"), dir.join("copies"))
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
    let never = crate::Switch::default();
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
) {
    let (work, copies) = scratch(name);
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
    (computers, work, made.id, here.id, confinement)
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
    let (computers, _work, _, _, confinement) = rt.block_on(fixture("property"));
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
    let (computers, _work, _, _, confinement) = rt.block_on(fixture("escapes"));
    assert_eq!(
        rt.block_on(confinement_escapes(&computers, &confinement)),
        Ok(())
    );
}

/// Swaps the folder `in_dir` for a link to the folder outside the workspace and back, the way
/// a command running beside the file tools could, until told to stop.
fn flip(inside: &std::path::Path, stop: &AtomicBool) {
    let (folder, parked) = (inside.join("in_dir"), inside.join("in_dir.parked"));
    while !stop.load(Ordering::Relaxed) {
        if std::fs::rename(&folder, &parked).is_ok() {
            let _ = std::os::unix::fs::symlink("../outside", &folder);
            let _ = std::fs::remove_file(&folder);
            let _ = std::fs::rename(&parked, &folder);
        }
    }
}

#[test]
fn a_link_swapped_between_the_check_and_the_open_never_reaches_outside() {
    let rt = runtime();
    let (computers, work, _, _, confinement) = rt.block_on(fixture("race"));
    let inside = work.join("confine/inside");
    std::fs::create_dir_all(inside.join("in_dir")).unwrap();
    std::fs::write(inside.join("in_dir/sentinel.txt"), "inside").unwrap();
    let outside_file = work.join("confine/outside/sentinel.txt");
    let stop = Arc::new(AtomicBool::new(false));
    let flipper = {
        let (inside, stop) = (inside.clone(), Arc::clone(&stop));
        std::thread::spawn(move || flip(&inside, &stop))
    };
    let workspace = confinement_workspace(&computers, &work, &rt);
    let started = std::time::Instant::now();
    let mut leaked_read = false;
    for _ in 0..4_000 {
        let write = FileWrite {
            path: "in_dir/sentinel.txt".into(),
            content: Bytes::new(b"PWNED"),
        };
        let _ = rt.block_on(computers.write(workspace, &[write]));
        if let Ok(bytes) = rt.block_on(computers.read(workspace, "in_dir/sentinel.txt")) {
            leaked_read |= bytes.as_slice() == b"SENTINEL";
        }
        if started.elapsed().as_secs() >= 4 {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    flipper.join().unwrap();
    drop(confinement);
    assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "SENTINEL");
    assert!(!leaked_read, "a read returned the outside file");
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

#[test]
fn commands_never_see_secret_variables() {
    let secrets = [
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
    for name in secrets {
        environment.push((name.into(), "hunter2".into()));
    }
    environment.push(("HARMLESS_SETTING".into(), "visible".into()));
    let (work, copies) = scratch("secrets");
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
        !seen.contains("hunter2"),
        "a secret reached a command: {seen}"
    );
    for name in secrets {
        assert!(!seen.contains(name), "{name} reached a command");
    }
}
