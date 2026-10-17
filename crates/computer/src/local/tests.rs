use std::path::PathBuf;

use glave_ports::testing::fake_secret;
use glave_ports::{Bytes, block_on, conformance};

use super::process::Utf8;
use super::*;

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
    let computers = LocalComputers::new(&copies, safe_environment())
        .with_worktrees(copies.with_file_name("worktrees"))
        .with_projects(copies.with_file_name("projects"));
    let verdict = block_on!(conformance::computers(&computers, &work.to_string_lossy()));
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
    block_on!(async {
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

/// Output that cancels the command once it has said its child is running.
struct CancelsWhenUp(Collect, crate::Switch);

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
    let (work, copies) = scratch("cancel-group");
    let computers = LocalComputers::new(&copies, safe_environment());
    block_on!(async {
        let made = computers.create(&Environment::host()).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        let here = computers
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        let cancel = crate::Switch::default();
        let output = CancelsWhenUp(Collect::default(), cancel.clone());
        // One child that dies with bash, one that ignores SIGTERM and needs SIGKILL.
        let script = "sleep 30 & echo $! > plain.pid; (trap '' TERM; exec sleep 30) & \
            echo $! > stubborn.pid; echo up; wait";
        let finished = computers
            .run(here.id, &Command::new(script), &output, &cancel)
            .await
            .unwrap();
        assert_eq!(finished.ending, glave_ports::Ending::Cancelled);
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

/// Every file under `dir`, read as bytes.
fn every_file(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(every_file(&path));
        } else if let Ok(bytes) = std::fs::read(&path) {
            found.push((path, bytes));
        }
    }
    found
}

/// Answers git over HTTP with "401, send a password" and records each Authorization header.
async fn asks_for_a_password(listener: tokio::net::TcpListener) -> Vec<String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut seen = Vec::new();
    for _ in 0..2 {
        let Ok((mut stream, _)) = listener.accept().await else {
            break;
        };
        let mut request = vec![0u8; 8192];
        let got = stream.read(&mut request).await.unwrap_or(0);
        let text = String::from_utf8_lossy(&request[..got]).into_owned();
        seen.extend(
            text.lines()
                .filter_map(|line| line.strip_prefix("Authorization: "))
                .map(str::to_owned),
        );
        let _ = stream
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"r\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
    }
    seen
}

#[test]
fn a_credential_reaches_git_through_its_helper_and_never_a_disk() {
    let (work, copies) = scratch("credential");
    let secret = fake_secret("");
    let credential = GitCredential::new("glave-user", &secret).unwrap();
    let basic = serde_json::to_value(Bytes::new(format!("glave-user:{secret}"))).unwrap();
    let basic = format!("Basic {}", basic.as_str().unwrap());
    let origin = work.join("origin.git");
    let script = format!(
        "git init -q --bare {o} && git init -q src && cd src && echo hi > a.txt && git add a.txt \
         && git -c user.name=t -c user.email=t@example.com commit -q -m one && \
         git push -q {o} HEAD:refs/heads/main",
        o = origin.display()
    );
    let made = std::process::Command::new("bash")
        .args(["-c", &script])
        .current_dir(&work)
        .status()
        .unwrap();
    assert!(made.success());
    let computers = LocalComputers::new(&copies, safe_environment());
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap();
        // Over HTTP, git asks the helper, and sends exactly the credential.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/r.git", listener.local_addr().unwrap());
        let server = tokio::spawn(asks_for_a_password(listener));
        let sources = Sources::Git { url, at: None };
        let refused = computers
            .open_with_credential(computer.id, &sources, &credential)
            .await;
        assert!(refused.is_err(), "the server never lets it in");
        let seen = server.await.unwrap();
        assert!(seen.contains(&basic), "{seen:?}");

        // A clone made with it, saved and pushed with it, holds it nowhere.
        let sources = Sources::Git {
            url: origin.to_string_lossy().into_owned(),
            at: Some("main".into()),
        };
        let opened = computers
            .open_with_credential(computer.id, &sources, &credential)
            .await
            .unwrap();
        std::fs::write(std::path::Path::new(&opened.root).join("b.txt"), "two").unwrap();
        let save = glave_ports::Save {
            branch: "glave/wip/test".into(),
            message: "work".into(),
        };
        assert!(computers.save(opened.id, &save).await.unwrap().pushed);
        let output = Collect::default();
        computers
            .run(
                opened.id,
                &Command::new("env; git config --list"),
                &output,
                &crate::Switch::default(),
            )
            .await
            .unwrap();
        let seen = output.0.lock().unwrap().clone();
        assert!(!seen.contains(&secret), "commands never see it: {seen}");
        for (path, bytes) in every_file(&work).into_iter().chain(every_file(&copies)) {
            assert!(
                !String::from_utf8_lossy(&bytes).contains(&secret),
                "{} holds the secret",
                path.display()
            );
        }
        assert!(!format!("{opened:?}").contains(&secret));
    });
}
