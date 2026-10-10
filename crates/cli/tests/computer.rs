//! `glave computer serve --stdio`, the real binary, as a harness elsewhere would reach it: the
//! computers suite runs against it across the protocol, and its commands never see glave's
//! secrets.

#[cfg(test)]
mod computer_tests {
    use std::process::Stdio;
    use std::sync::Mutex;

    use glave_computer::RemoteComputers;
    use glave_ports::{
        BoxFuture, Cancellation, Command, Computers, Ending, Environment, Output, Sources,
        conformance,
    };

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[derive(Default)]
    struct Collect(Mutex<String>);

    impl Output for Collect {
        fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
            self.0.lock().unwrap().push_str(text);
            Box::pin(std::future::ready(()))
        }
    }

    struct Never;

    impl Cancellation for Never {
        fn is_cancelled(&self) -> bool {
            false
        }
        fn cancelled(&self) -> BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }

    #[test]
    fn the_binary_serves_a_computer_that_passes_the_suite() {
        let dir = std::env::temp_dir().join(format!("glave-e2e-computer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        block_on(async {
            let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_glave"))
                .args(["computer", "serve", "--stdio"])
                .env("GLAVE_HOME", dir.join("home"))
                .env("GLAVE_API_KEY", "sk-must-not-leak")
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
                .run(here.id, &Command::new("env"), &seen, &Never)
                .await
                .unwrap();
            assert_eq!(finished.ending, Ending::Exited { code: 0 });
            let seen = seen.0.lock().unwrap().clone();
            assert!(seen.contains("PATH="), "{seen}");
            assert!(!seen.contains("sk-must-not-leak"), "{seen}");
            remote.destroy(made.id).await.unwrap();

            let suite = dir.join("suite");
            std::fs::create_dir_all(&suite).unwrap();
            let verdict = conformance::computers(&remote, &suite.to_string_lossy()).await;
            assert_eq!(verdict, Ok(()));
            drop(remote);
            let status = child.wait().await.unwrap();
            assert!(
                status.success(),
                "the server ends cleanly when its input does"
            );
        });
    }
}
