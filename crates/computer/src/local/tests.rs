use std::path::PathBuf;

use glave_ports::computer::{Image, Network};
use glave_ports::{block_on, conformance};

use super::process::Utf8;
use super::*;

/// An empty folder for one test, and another for the copies it makes.
fn scratch(name: &str) -> (glave_ports::testing::TempDir, PathBuf, PathBuf) {
    let dir = glave_ports::testing::TempDir::new(&format!("local-{name}")).unwrap();
    std::fs::create_dir_all(dir.join("work")).unwrap();
    let (work, copies) = (dir.join("work"), dir.join("copies"));
    (dir, work, copies)
}

fn safe_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect()
}

#[test]
fn this_machine_passes_the_computers_suite() {
    let (_dir, work, copies) = scratch("suite");
    let computers = LocalComputers::new(&copies, safe_environment())
        .with_worktrees(copies.with_file_name("worktrees"))
        .with_projects(copies.with_file_name("projects"));
    let verdict = block_on!(conformance::computers(
        &computers,
        &Environment::host(),
        &work.to_string_lossy()
    ));
    assert_eq!(verdict, Ok(()));
    let left = std::fs::read_dir(&copies).map_or(0, Iterator::count);
    assert_eq!(left, 0, "destroying the computer removes its copies");
}

#[test]
fn commands_see_this_machine_and_the_computer_but_nothing_else() {
    let (_dir, work, copies) = scratch("env");
    let computers = LocalComputers::new(&copies, safe_environment());
    let mut environment = Environment::host();
    environment
        .env
        .insert("FROM_ENVIRONMENT".into(), "yes".into());
    block_on!(async {
        let made = computers.create(&environment).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        let here = computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        let output = Collect::default();
        let never = crate::CancelSwitch::default();
        computers
            .run(here.id, &Command::new("env"), &output, &never)
            .await
            .unwrap();
        let seen = output.0.lock().unwrap().clone();
        assert!(seen.contains("FROM_ENVIRONMENT=yes"), "{seen}");
        assert!(seen.contains("PATH="), "{seen}");
        assert!(!seen.contains("GLAVE_API_KEY"), "{seen}");
    });
}

#[derive(Default)]
struct Collect(Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}

/// Output that cancels the command once it has said its child is running.
struct CancelsWhenUp(Collect, crate::CancelSwitch);

impl Output for CancelsWhenUp {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        let written = self.0.write(text);
        if self.0.0.lock().is_ok_and(|all| all.contains("up")) {
            self.1.cancel();
        }
        written
    }
}

#[cfg(unix)]
#[test]
fn a_cancelled_command_returns_only_once_everything_it_started_has_gone() {
    let (_dir, work, copies) = scratch("cancel-group");
    let computers = LocalComputers::new(&copies, safe_environment());
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        let here = computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        let cancel = crate::CancelSwitch::default();
        let output = CancelsWhenUp(Collect::default(), cancel.clone());
        // One child that dies with bash, one that ignores SIGTERM and needs SIGKILL.
        let script = "sleep 30 & echo $! > plain.pid; (trap '' TERM; exec sleep 30) & \
            echo $! > stubborn.pid; echo up; wait";
        let finished = computers
            .run(here.id, &Command::new(script), &output, &cancel)
            .await
            .unwrap();
        assert_eq!(finished.ending, glave_ports::computer::Ending::Cancelled);
        for name in ["plain.pid", "stubborn.pid"] {
            let pid = std::fs::read_to_string(work.join(name)).unwrap();
            let alive = std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(!alive.success(), "{name} was stopped before run returned");
        }
    });
}

#[test]
fn what_the_host_cannot_honour_is_refused() {
    let (_dir, _, copies) = scratch("refuse");
    let computers = LocalComputers::new(&copies, safe_environment());
    for environment in [
        Environment {
            image: Image::Oci("rust:1".into()),
            ..Environment::host()
        },
        Environment {
            setup: vec!["make".into()],
            ..Environment::host()
        },
        Environment {
            network: Some(Network::Off),
            ..Environment::host()
        },
    ] {
        let refused = block_on!(computers.create(&environment));
        assert!(
            matches!(refused, Err(ComputerError::Unsupported(_))),
            "{refused:?}"
        );
    }
}

#[test]
fn characters_are_never_split_across_chunks() {
    let mut text = Utf8::default();
    let bytes = "héllo ✓".as_bytes();
    let mut all = String::new();
    for byte in bytes {
        all.push_str(&text.push(std::slice::from_ref(byte)));
    }
    all.push_str(&text.finish());
    assert_eq!(all, "héllo ✓");
    assert_eq!(text.push(&[0xff, b'a']), "\u{fffd}a");
}
