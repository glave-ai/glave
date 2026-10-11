//! A computer across the protocol passes the same suite as one in-process: over an
//! in-memory pipe, over TCP on 127.0.0.1 with a token, and over TLS with the certificate
//! pinned and a token that expires.

#[cfg(test)]
mod remote_tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use glave_computer::{
        Identity, LocalComputers, RemoteComputers, Token, connect_tls, serve, serve_tcp, serve_tls,
    };
    use glave_ports::{ComputerError, ComputerId, Computers, conformance};
    use tokio::io::BufReader;
    use tokio::net::{TcpListener, TcpStream};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("glave-remote-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("work")).unwrap();
        (dir.join("work"), dir.join("copies"))
    }

    fn local(copies: &PathBuf) -> Arc<LocalComputers> {
        let environment = std::env::vars_os().filter(|(name, _)| name == "PATH" || name == "HOME");
        Arc::new(
            LocalComputers::new(copies, environment)
                .with_worktrees(copies.with_file_name("worktrees")),
        )
    }

    #[test]
    fn a_computer_across_a_pipe_passes_the_suite() {
        let (work, copies) = scratch("pipe");
        block_on(async {
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

    #[test]
    fn a_computer_over_tcp_passes_the_suite_once_signed_in() {
        let (work, copies) = scratch("tcp");
        block_on(async {
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
        let (work, copies) = scratch("tls");
        block_on(async {
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
        let (_, copies) = scratch("tls-expired");
        block_on(async {
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
}
