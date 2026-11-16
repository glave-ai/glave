//! `glave computer serve --stdio`, the real binary, as a harness elsewhere would reach it: the
//! computers suite runs against it across the protocol, and its commands never see glave's
//! secrets. And `--tls`, ending when its input does and hiding its folder from its sandboxed
//! commands, and `--socket` as a computer in a Pod runs it, reached through bridges that each
//! end alone.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use glave_computer::{Identity, RemoteComputers, Token, connect_tls};
use glave_ports::computer::{
    Command, Computers, Ending, Environment, Output, Sandbox, SandboxLevel, SandboxNetwork, Sources,
};
use glave_ports::testing::{SocketPath, TempDir, any_secret, read_program};
use glave_ports::{BoxFuture, Clock as _, NeverCancelled, SystemClock, block_on, conformance};

use crate::support::Lifeline;

const GLAVE: &str = env!("CARGO_BIN_EXE_glave");

#[derive(Default)]
struct Collect(Mutex<String>);

impl Output for Collect {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        self.0.lock().unwrap().push_str(text);
        Box::pin(std::future::ready(()))
    }
}

/// `glave computer serve --stdio` with `env` over this test's own, and the client reaching it,
/// its protocol checked.
pub(crate) async fn served_on_stdio(
    env: &[(&str, &std::ffi::OsStr)],
) -> (tokio::process::Child, RemoteComputers) {
    let mut child = tokio::process::Command::new(GLAVE)
        .args(["computer", "serve", "--stdio"])
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let input = child.stdout.take().unwrap();
    let output = child.stdin.take().unwrap();
    let remote = RemoteComputers::connect(input, output);
    remote.check_protocol().await.unwrap();
    (child, remote)
}

#[test]
fn the_binary_serves_a_computer_that_passes_the_suite() {
    let dir = TempDir::new("e2e-computer").unwrap();
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let key = any_secret();
    read_program(GLAVE).unwrap();
    block_on!(async {
        let home = dir.join("home");
        let env = [
            ("GLAVE_HOME", home.as_os_str()),
            ("GLAVE_API_KEY", key.as_ref()),
        ];
        let (mut child, remote) = served_on_stdio(&env).await;

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
        assert!(!seen.contains(&key), "glave's key reached a command");
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

/// `dir/tls`, holding a new identity and a token, as `glave computer serve --tls` reads them.
fn tls_folder(dir: &Path) -> (PathBuf, Identity, Token) {
    let tls = dir.join("tls");
    std::fs::create_dir_all(&tls).unwrap();
    let identity = Identity::mint().unwrap();
    std::fs::write(tls.join("cert.pem"), &identity.certificate).unwrap();
    std::fs::write(tls.join("key.pem"), &identity.key).unwrap();
    let now = SystemTime::from(SystemClock.now());
    let token = Token::mint(now, Duration::from_mins(10)).unwrap();
    std::fs::write(tls.join("token"), token.to_file()).unwrap();
    (tls, identity, token)
}

/// `glave computer serve --tls` at `address` with the identity and token in `tls`, glave's
/// folder in `dir`, ending with its input.
fn serving_tls(dir: &Path, address: &str, tls: &Path) -> tokio::process::Child {
    tokio::process::Command::new(GLAVE)
        .args(["computer", "serve", "--tls", address])
        .arg(tls)
        .arg("--until-input-ends")
        .env("GLAVE_HOME", dir.join("home"))
        .stdin(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[test]
fn a_glave_serving_tls_ends_when_its_input_does() {
    let dir = TempDir::new("e2e-computer-tls").unwrap();
    let (tls, _, _) = tls_folder(&dir);
    read_program(GLAVE).unwrap();
    block_on!(async {
        let mut child = serving_tls(&dir, "127.0.0.1:0", &tls);
        // What the glave that started it holds, closed as that glave ends, however it ends.
        drop(child.stdin.take());
        let ended = tokio::time::timeout(Duration::from_secs(30), child.wait()).await;
        let status = ended.expect("it ends within seconds of its input").unwrap();
        assert!(status.success(), "{status}");
    });
}

/// A computer served with TLS on a port of this computer free a moment ago, from `dir` as
/// [`serving_tls`] serves it, and the client reaching it once it serves. Another port when
/// another program took that one first.
async fn served_on_tls(
    dir: &Path,
    tls: &Path,
    identity: &Identity,
    token: &Token,
) -> (tokio::process::Child, RemoteComputers) {
    for _ in 0..3 {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = free.local_addr().unwrap();
        drop(free);
        let mut child = serving_tls(dir, &address.to_string(), tls);
        // Up to 30 seconds for it to start listening.
        for _ in 0..600 {
            if let Ok(remote) = connect_tls(address, &identity.fingerprint, token.secret()).await {
                return (child, remote);
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    panic!("glave never served TLS");
}

/// A computer served with TLS keeps its folder from its sandboxed commands, as a computer on a
/// socket keeps its own (#1502): with the token there, a command with the network could reach
/// the computer and open a terminal outside every sandbox, and with the key pass as it. What
/// sits beside the folder stays readable, and the same command outside the sandbox reads both,
/// so the test cannot pass by being broken.
#[test]
fn a_tls_computer_s_token_and_key_are_hidden_from_its_sandboxed_commands() {
    if !glave_computer::sandbox_available() {
        assert!(
            std::env::var_os("GLAVE_REQUIRE_SANDBOX").is_none(),
            "GLAVE_REQUIRE_SANDBOX is set but the sandbox cannot run here"
        );
        return;
    }
    let dir = TempDir::outside_temp("computer-tls-hidden").unwrap();
    let (tls, identity, token) = tls_folder(&dir);
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let beside = dir.join("beside");
    std::fs::write(&beside, "beside-ok\n").unwrap();
    let reads = format!(
        "cat '{0}/token' '{0}/key.pem' '{1}'; true",
        tls.display(),
        beside.display()
    );
    read_program(GLAVE).unwrap();
    block_on!(async {
        let (mut child, remote) = served_on_tls(&dir, &tls, &identity, &token).await;
        let made = remote.create(&Environment::host()).await.unwrap();
        let path = work.to_string_lossy().into_owned();
        let here = remote
            .open(made.id, &Sources::Folder { path })
            .await
            .unwrap();
        let read = async |sandbox: Option<Sandbox>| {
            let mut command = Command::new(reads.as_str());
            command.sandbox = sandbox;
            let seen = Collect::default();
            let finished = remote.run(here.id, &command, &seen, &NeverCancelled).await;
            assert_eq!(finished.unwrap().ending, Ending::Exited { code: 0 });
            seen.0.into_inner().unwrap()
        };
        let sandbox = Sandbox {
            level: SandboxLevel::Project,
            network: SandboxNetwork::Direct,
            home: Some(dir.to_path_buf()),
            glave_home: Some(dir.join("home")),
        };
        let said = read(Some(sandbox)).await;
        assert!(said.contains("beside-ok"), "{said}");
        // What was read is not printed: it would be the token or the key.
        assert!(
            !said.contains(token.secret()),
            "a sandboxed command read the token"
        );
        assert!(
            !said.contains("PRIVATE KEY"),
            "a sandboxed command read the key"
        );
        let said = read(None).await;
        assert!(said.contains(token.secret()), "outside, the token is read");
        assert!(said.contains("PRIVATE KEY"), "outside, the key is read");
        remote.destroy(made.id).await.unwrap();
        drop(remote);
        drop(child.stdin.take());
        let ended = tokio::time::timeout(Duration::from_secs(30), child.wait()).await;
        assert!(ended.expect("it ends with its input").unwrap().success());
    });
}

#[test]
fn a_computer_on_a_socket_is_reached_through_bridges_that_each_end_alone() {
    let dir = TempDir::new("e2e-computer-socket").unwrap();
    let secret = dir.join("secret");
    std::fs::create_dir_all(&secret).unwrap();
    let now = SystemTime::from(SystemClock.now());
    let token = Token::mint(now, Duration::from_mins(10)).unwrap();
    std::fs::write(secret.join("token"), token.to_file()).unwrap();
    let socket = SocketPath::new(&dir, "glave.sock").unwrap();
    // Each glave ends with the test, however it ends.
    let lifeline = Lifeline::new(&dir);
    read_program(GLAVE).unwrap();
    let glave = || {
        let mut command = tokio::process::Command::new(GLAVE);
        command.env(Lifeline::VARIABLE, &lifeline.path);
        command
    };
    block_on!(async {
        let mut server = glave()
            .args(["computer", "serve", "--socket"])
            .arg(&*socket)
            .arg(&secret)
            .env("GLAVE_HOME", dir.join("home"))
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        for _ in 0..2 {
            let mut bridge = glave()
                .args(["computer", "bridge"])
                .arg(&*socket)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let (input, output) = (bridge.stdout.take().unwrap(), bridge.stdin.take().unwrap());
            let remote = RemoteComputers::connect(input, output);
            remote.authenticate(token.secret()).await.unwrap();
            remote.check_protocol().await.unwrap();
            // Its input closed, as an exec that drops closes it: the bridge ends, and only it.
            drop(remote);
            let ended = tokio::time::timeout(Duration::from_secs(30), bridge.wait()).await;
            let status = ended
                .expect("a bridge ends within seconds of its input")
                .unwrap();
            assert!(status.success(), "{status}");
        }
        let running = server.try_wait().unwrap();
        server.kill().await.unwrap();
        assert!(running.is_none(), "the computer serves on: {running:?}");
    });
}
