//! A computer across the protocol passes the same suite as one in-process: over an
//! in-memory pipe, over TCP on 127.0.0.1 with a token, and over TLS with the certificate
//! pinned and a token that expires.

use std::path::PathBuf;
use std::sync::Arc;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use glave_computer::{
    Fingerprint, Identity, LocalComputers, RemoteComputers, Token, connect_tls, serve, serve_tcp,
    serve_tls,
};
use glave_ports::computer::{ComputerError, ComputerId, Computers, Environment};
use glave_ports::{Clock as _, SystemClock, block_on, conformance};
use tokio::io::{AsyncBufReadExt as _, BufReader};
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
        let (remote, serving) = across_a_pipe(local(&copies));
        remote.check_protocol().await.unwrap();
        let verdict =
            conformance::computers(&remote, &Environment::host(), &work.to_string_lossy()).await;
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
            matches!(&refused, Err(ComputerError::Unsupported(m)) if m.contains("2026-10-28")),
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
        let verdict =
            conformance::computers(&remote, &Environment::host(), &work.to_string_lossy()).await;
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

/// `computers` served over an in-memory pipe, a client of them, and the serving, which ends
/// once the client is dropped.
fn across_a_pipe(
    computers: Arc<LocalComputers>,
) -> (
    RemoteComputers,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server);
    let serving = tokio::spawn(serve(
        computers,
        BufReader::new(server_read),
        server_write,
        None,
    ));
    let (client_read, client_write) = tokio::io::split(client);
    (RemoteComputers::connect(client_read, client_write), serving)
}

/// What `script` printed, run on a new computer of `remote` in `work`.
async fn printed<C: Computers>(remote: &C, work: &std::path::Path, script: &str) -> String {
    use glave_ports::computer::{Command, Sources};
    let made = remote.create(&Environment::host()).await.unwrap();
    let path = work.to_string_lossy().into_owned();
    let here = remote
        .open(made.id, &Sources::Folder { path })
        .await
        .unwrap();
    let output = glave_ports::testing::Captured::default();
    let never = glave_computer::CancelSwitch::default();
    remote
        .run(here.id, &Command::new(script), &output, &never)
        .await
        .unwrap();
    output.text()
}

/// Runs `env` on `remote` in `work`: ordinary settings come through, secrets never do.
async fn secrets_stay_out<C: Computers>(remote: &C, work: &std::path::Path) {
    let seen = printed(remote, work, "env").await;
    assert!(seen.contains("HARMLESS_SETTING=visible"), "{seen}");
    assert!(
        !seen.contains("hunter2"),
        "a secret reached a command: {seen}"
    );
    for name in SECRETS {
        assert!(!seen.contains(name), "{name} reached a command");
    }
}

/// A served computer, as a worker is, puts its own glave first on its commands' `PATH`, so
/// they reach the glave serving them whatever the `PATH` they were given holds.
#[test]
fn a_computer_across_a_pipe_puts_its_glave_first_on_its_commands_path() {
    use std::os::unix::fs::PermissionsExt as _;

    let (dir, work, copies) = scratch("on-path");
    let serving_glave = dir.join("serving/glave");
    std::fs::create_dir_all(serving_glave.parent().unwrap()).unwrap();
    std::fs::write(&serving_glave, "#!/bin/sh\necho serving-glave\n").unwrap();
    std::fs::set_permissions(&serving_glave, std::fs::Permissions::from_mode(0o755)).unwrap();
    let on_path = dir.join("glave-home/bin");
    block_on!(async {
        glave_computer::glave_on_path(&on_path, &serving_glave)
            .await
            .unwrap();
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH");
        let computers = LocalComputers::new(&copies, environment).with_glave_on_path(&on_path);
        let (remote, serving) = across_a_pipe(Arc::new(computers));
        let seen = printed(&remote, &work, "glave").await;
        assert!(seen.contains("serving-glave"), "{seen}");
        drop(remote);
        serving.await.unwrap().unwrap();
    });
}

#[test]
fn a_computer_across_a_pipe_keeps_secret_variables_from_commands() {
    let (_dir, work, copies) = scratch("secrets-pipe");
    block_on!(async {
        let (remote, serving) = across_a_pipe(with_secrets(&copies));
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
        let token = Token::mint(
            SystemTime::from(SystemClock.now()),
            Duration::from_secs(600),
        )
        .unwrap();
        let (address, identity, serving) = tls(&copies, token.clone()).await;
        let remote = connect_tls(address, &identity.fingerprint, token.secret())
            .await
            .unwrap();
        let verdict =
            conformance::computers(&remote, &Environment::host(), &work.to_string_lossy()).await;
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
fn a_client_that_never_signs_in_is_told_so_and_let_go_in_seconds() {
    let (_dir, _, copies) = scratch("tcp-silent");
    block_on!(async {
        // The clock jumps to each limit as soon as nothing else can run.
        tokio::time::pause();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let serving = tokio::spawn(serve_tcp(local(&copies), listener, "s3cret".into()));
        let mut silent = BufReader::new(TcpStream::connect(address).await.unwrap());
        let started = tokio::time::Instant::now();
        let mut said = String::new();
        silent.read_line(&mut said).await.unwrap();
        let waited = started.elapsed();
        assert!(Duration::from_secs(10) <= waited && waited < Duration::from_secs(11));
        assert!(
            said.contains("no connection/authenticate came within 10 seconds"),
            "{said}"
        );
        let mut rest = String::new();
        assert_eq!(
            silent.read_line(&mut rest).await.unwrap(),
            0,
            "it is closed"
        );
        serving.abort();
    });
}

#[test]
fn a_computer_that_takes_the_connection_and_says_nothing_is_given_up_in_seconds() {
    block_on!(async {
        tokio::time::pause();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        // It takes the connection and says nothing, as a stuck computer would.
        let stuck = tokio::spawn(async move {
            let _held = listener.accept().await;
            std::future::pending::<()>().await;
        });
        let fingerprint = Fingerprint::parse(&"ab".repeat(32)).unwrap();
        let started = tokio::time::Instant::now();
        let tried = connect_tls(address, &fingerprint, "s3cret").await;
        // On a paused clock the limit on reaching the port may pass first; either says so.
        assert!(
            matches!(&tried, Err(ComputerError::Unreachable(m)) if m.ends_with("within 10 seconds")),
            "{:?}",
            tried.err()
        );
        assert!(started.elapsed() < Duration::from_secs(21));
        stuck.abort();
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
