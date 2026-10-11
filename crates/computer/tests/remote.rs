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
        use glave_ports::{BoxFuture, Command, Environment, Output, Sources};
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
        let never = glave_computer::Switch::default();
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
        let (work, copies) = scratch("secrets-pipe");
        block_on(async {
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
        let (work, copies) = scratch("secrets-tcp");
        block_on(async {
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
}
