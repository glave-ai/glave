//! `glave computer serve --stdio`, the real binary, as a harness elsewhere would reach it: the
//! computers suite runs against it across the protocol, and its commands never see glave's
//! secrets. And `--tls` as a worker runs it, ending when its input does.

use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use glave_computer::{Identity, RemoteComputers, Token};
use glave_ports::computer::{Command, Computers, Ending, Environment, Output, Sources};
use glave_ports::testing::{FAKE_SECRET, TempDir};
use glave_ports::{BoxFuture, Clock as _, NeverCancelled, SystemClock, block_on, conformance};

#[derive(Default)]
struct Collect(Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push_str(text);
        Box::pin(std::future::ready(()))
    }
}

#[test]
fn the_binary_serves_a_computer_that_passes_the_suite() {
    let dir = TempDir::new("e2e-computer").unwrap();
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    block_on!(async {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_glave"))
            .args(["computer", "serve", "--stdio"])
            .env("GLAVE_HOME", dir.join("home"))
            .env("GLAVE_API_KEY", FAKE_SECRET)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdout.take().unwrap();
        let output = child.stdin.take().unwrap();
        let remote = RemoteComputers::connect(input, output);
        remote.check_protocol().await.unwrap();

        let made = remote.create(&Environment::host()).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        let here = remote
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        let seen = Collect::default();
        let finished = remote
            .run(here.id, &Command::new("env"), &seen, &NeverCancelled)
            .await
            .unwrap();
        assert_eq!(finished.ending, Ending::Exited { code: 0 });
        let seen = seen.0.lock().unwrap().clone();
        assert!(seen.contains("PATH="), "{seen}");
        assert!(!seen.contains(FAKE_SECRET), "glave's key reached a command");
        remote.destroy(made.id).await.unwrap();

        let suite = dir.join("suite");
        std::fs::create_dir_all(&suite).unwrap();
        let verdict =
            conformance::computers(&remote, &Environment::host(), &suite.to_string_lossy()).await;
        assert_eq!(verdict, Ok(()));
        drop(remote);
        let status = child.wait().await.unwrap();
        assert!(
            status.success(),
            "the server ends cleanly when its input does"
        );
    });
}

#[test]
fn a_worker_s_glave_serving_tls_ends_when_its_input_does() {
    let dir = TempDir::new("e2e-computer-tls").unwrap();
    let tls = dir.join("tls");
    std::fs::create_dir_all(&tls).unwrap();
    let identity = Identity::mint().unwrap();
    std::fs::write(tls.join("cert.pem"), &identity.certificate).unwrap();
    std::fs::write(tls.join("key.pem"), &identity.key).unwrap();
    let now = SystemTime::from(SystemClock.now());
    let token = Token::mint(now, Duration::from_mins(10)).unwrap();
    std::fs::write(tls.join("token"), token.to_file()).unwrap();
    block_on!(async {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_glave"))
            .args(["computer", "serve", "--tls", "127.0.0.1:0"])
            .arg(&tls)
            .arg("--until-input-ends")
            .env("GLAVE_HOME", dir.join("home"))
            .stdin(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        // What the glave that started it holds, closed as that glave ends, however it ends.
        drop(child.stdin.take());
        let ended = tokio::time::timeout(Duration::from_secs(30), child.wait()).await;
        let status = ended.expect("it ends within seconds of its input").unwrap();
        assert!(status.success(), "{status}");
    });
}
