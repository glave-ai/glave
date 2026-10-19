//! A computer across the protocol passes the same suite as one in-process: over an
//! in-memory pipe, over TCP on 127.0.0.1 with a token, and over TLS with the certificate
//! pinned and a token that expires.

use std::path::PathBuf;
use std::sync::Arc;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use glave_computer::{
    Identity, LocalComputers, RemoteComputers, Token, connect_tls, serve, serve_tcp, serve_tls,
};
use glave_ports::computer::{ComputerError, ComputerId, Computers};
use glave_ports::{block_on, conformance};
use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};

fn scratch(name: &str) -> (glave_ports::testing::TempDir, PathBuf, PathBuf) {
    let dir = glave_ports::testing::TempDir::new(&format!("remote-{name}")).unwrap();
    std::fs::create_dir_all(dir.join("work")).unwrap();
    let (work, copies) = (dir.join("work"), dir.join("copies"));
    (dir, work, copies)
}

fn local(copies: &PathBuf) -> Arc<LocalComputers> {
    let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
    Arc::new(
        LocalComputers::new(copies, environment)
            .with_worktrees(copies.with_file_name("worktrees"))
            .with_projects(copies.with_file_name("projects")),
    )
}

#[test]
fn a_computer_across_a_pipe_passes_the_suite() {
    let (_dir, work, copies) = scratch("pipe");
    block_on!(async {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server);
        let serving = tokio::spawn(serve(
            local(&copies),
            BufReader::new(server_read),
            server_write,
            None,
        ));
        let (client_read, client_write) = tokio::io::split(client);
        let remote = RemoteComputers::connect(client_read, client_write);
        remote.check_protocol().await.unwrap();
        let verdict = conformance::computers(&remote, &work.to_string_lossy()).await;
        assert_eq!(verdict, Ok(()));
        drop(remote);
        serving.await.unwrap().unwrap();
    });
}

/// A server from before protected writes (`2026-10-10.1`) would ignore `protect` and
/// write, so the client refuses it before sending anything: it fails closed.
#[test]
fn a_server_that_predates_protected_writes_is_refused() {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
    block_on!(async {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let older = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let answer = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": {"name": "glave-computer", "protocols": ["2026-10-10.1"]},
                });
                write
                    .write_all(format!("{answer}\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        let (read, write) = tokio::io::split(client);
        let remote = RemoteComputers::connect(read, write);
        let refused = remote.check_protocol().await;
        assert!(
            matches!(&refused, Err(ComputerError::Unsupported(m)) if m.contains("2026-10-17")),
            "{refused:?}"
        );
        drop(remote);
        older.await.unwrap();
    });
}

#[test]
fn a_computer_over_tcp_passes_the_suite_once_signed_in() {
    let (_dir, work, copies) = scratch("tcp");
    block_on!(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let serving = tokio::spawn(serve_tcp(local(&copies), listener, "s3cret".into()));

        let (read, write) = TcpStream::connect(address).await.unwrap().into_split();
        let stranger = RemoteComputers::connect(read, write);
        let refused = stranger.authenticate("guess").await;
        assert!(
            matches!(refused, Err(ComputerError::Unreachable(_))),
            "{:?}",
            refused.err()
        );
        let after = stranger
            .status(ComputerId::from_uuid(uuid::Uuid::nil()))
            .await;
        assert!(after.is_err(), "a refused connection is closed");

        let (read, write) = TcpStream::connect(address).await.unwrap().into_split();
        let remote = RemoteComputers::connect(read, write);
        remote.authenticate("s3cret").await.unwrap();
        remote.check_protocol().await.unwrap();
        let verdict = conformance::computers(&remote, &work.to_string_lossy()).await;
        assert_eq!(verdict, Ok(()));
        serving.abort();
    });
}

/// Names of secret variables in every case, set to one value no command may print.
const SECRETS: [&str; 5] = [
    "OPENAI_API_KEY",
    "glave_api_key",
    "Github_Token",
    "DB_PASSWORD",
    "my_passwd",
];

/// This machine as a computer, given the secret variables and one ordinary setting.
fn with_secrets(copies: &PathBuf) -> Arc<LocalComputers> {
    let mut environment: Vec<_> = std::env::vars_os()
        .filter(|(name, _)| name == "PATH" || name == "HOME")
        .collect();
    for name in SECRETS {
        environment.push((name.into(), "hunter2".into()));
    }
    environment.push(("HARMLESS_SETTING".into(), "visible".into()));
    Arc::new(LocalComputers::new(copies, environment))
}

/// Runs `env` on `remote` in `work`: ordinary settings come through, secrets never do.
async fn secrets_stay_out<C: Computers>(remote: &C, work: &std::path::Path) {
    use glave_ports::BoxFuture;
    use glave_ports::computer::{Command, Environment, Output, Sources};
    use std::sync::Mutex;

    struct Text(Mutex<String>);
    impl Output for Text {
        fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
            if let Ok(mut all) = self.0.lock() {
                all.push_str(text);
            }
            Box::pin(std::future::ready(()))
        }
    }

    let made = remote.create(&Environment::host()).await.unwrap();
    let path = work.to_string_lossy().into_owned();
    let here = remote
        .open(made.id, &Sources::Folder { path })
        .await
        .unwrap();
    let text = Text(Mutex::default());
    let never = glave_computer::CancelSwitch::default();
    remote
        .run(here.id, &Command::new("env"), &text, &never)
        .await
        .unwrap();
    let seen = text.0.lock().unwrap().clone();
    assert!(seen.contains("HARMLESS_SETTING=visible"), "{seen}");
    assert!(
        !seen.contains("hunter2"),
        "a secret reached a command: {seen}"
    );
    for name in SECRETS {
        assert!(!seen.contains(name), "{name} reached a command");
    }
}

#[test]
fn a_computer_across_a_pipe_keeps_secret_variables_from_commands() {
    let (_dir, work, copies) = scratch("secrets-pipe");
    block_on!(async {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server);
        let serving = tokio::spawn(serve(
            with_secrets(&copies),
            BufReader::new(server_read),
            server_write,
            None,
        ));
        let (client_read, client_write) = tokio::io::split(client);
        let remote = RemoteComputers::connect(client_read, client_write);
        secrets_stay_out(&remote, &work).await;
        drop(remote);
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn a_computer_over_tcp_keeps_secret_variables_from_commands() {
    let (_dir, work, copies) = scratch("secrets-tcp");
    block_on!(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let serving = tokio::spawn(serve_tcp(with_secrets(&copies), listener, "s3cret".into()));
        let (read, write) = TcpStream::connect(address).await.unwrap().into_split();
        let remote = RemoteComputers::connect(read, write);
        remote.authenticate("s3cret").await.unwrap();
        secrets_stay_out(&remote, &work).await;
        serving.abort();
    });
}

/// A TLS listener on 127.0.0.1 serving `copies`, with a fresh identity and `token`.
async fn tls(copies: &PathBuf, token: Token) -> (std::net::SocketAddr, Identity, JoinHandle) {
    let identity = Identity::mint().unwrap();
    let acceptor = Identity::acceptor(&identity.certificate, &identity.key).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let serving = tokio::spawn(serve_tls(local(copies), listener, acceptor, token));
    (address, identity, serving)
}

type JoinHandle = tokio::task::JoinHandle<std::io::Result<()>>;

#[test]
fn a_computer_over_tls_passes_the_suite_with_its_certificate_pinned() {
    let (_dir, work, copies) = scratch("tls");
    block_on!(async {
        let token = Token::mint(SystemTime::now(), Duration::from_secs(600)).unwrap();
        let (address, identity, serving) = tls(&copies, token.clone()).await;
        let remote = connect_tls(address, &identity.fingerprint, token.secret())
            .await
            .unwrap();
        let verdict = conformance::computers(&remote, &work.to_string_lossy()).await;
        assert_eq!(verdict, Ok(()));

        let stranger = Identity::mint().unwrap();
        let refused = connect_tls(address, &stranger.fingerprint, token.secret()).await;
        assert!(
            matches!(&refused, Err(ComputerError::Unreachable(m)) if m.contains("handshake")),
            "another certificate is never trusted: {:?}",
            refused.err()
        );
        let guessed = connect_tls(address, &identity.fingerprint, "guess").await;
        assert!(guessed.is_err(), "a wrong token is refused");
        serving.abort();
    });
}

#[test]
fn an_expired_token_opens_no_connection() {
    let (_dir, _, copies) = scratch("tls-expired");
    block_on!(async {
        // It expired the moment it was made, at the start of Unix time.
        let token = Token::mint(UNIX_EPOCH, Duration::from_secs(1)).unwrap();
        let (address, identity, serving) = tls(&copies, token.clone()).await;
        let refused = connect_tls(address, &identity.fingerprint, token.secret()).await;
        assert!(
            matches!(&refused, Err(ComputerError::Unreachable(m)) if m.contains("valid token")),
            "{:?}",
            refused.err()
        );
        serving.abort();
    });
}
