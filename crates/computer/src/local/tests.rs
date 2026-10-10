use std::path::PathBuf;

use glave_ports::conformance;

use super::command::Utf8;
use super::*;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// An empty folder for one test, and another for the copies it makes.
fn scratch(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("glave-local-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    (dir.join("work"), dir.join("copies"))
}

fn safe_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect()
}

#[test]
fn this_machine_passes_the_computers_suite() {
    let (work, copies) = scratch("suite");
    let computers = LocalComputers::new(&copies, safe_environment());
    let verdict = block_on(conformance::computers(&computers, &work.to_string_lossy()));
    assert_eq!(verdict, Ok(()));
    let left = std::fs::read_dir(&copies).map_or(0, Iterator::count);
    assert_eq!(left, 0, "destroying the computer removes its copies");
}

#[test]
fn commands_see_this_machine_and_the_computer_but_nothing_else() {
    let (work, copies) = scratch("env");
    let computers = LocalComputers::new(&copies, safe_environment());
    let mut environment = Environment::host();
    environment
        .env
        .insert("FROM_ENVIRONMENT".into(), "yes".into());
    block_on(async {
        let made = computers.create(&environment).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        let here = computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        let output = Collect::default();
        let never = crate::Switch::default();
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
struct Collect(std::sync::Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}

#[test]
fn what_the_host_cannot_honour_is_refused() {
    let (_, copies) = scratch("refuse");
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
        let refused = block_on(computers.create(&environment));
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
