//! A computer across the protocol passes the same suite as one in-process: over an
//! in-memory pipe, and over TCP on 127.0.0.1 with a token.

#[cfg(test)]
mod remote_tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use glave_computer::{LocalComputers, RemoteComputers, serve, serve_tcp};
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
        Arc::new(LocalComputers::new(copies, environment))
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
                "{refused:?}"
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
}
