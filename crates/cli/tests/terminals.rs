//! A person's terminals, opened by glave's own program, which starts their shell in a session
//! of its own: this machine's computers pass the terminals suite with the real binary as their
//! helper, started directly and through the lifeline's gate, and so does
//! `glave computer serve --stdio`, across the protocol.

use std::process::Stdio;

use glave_computer::{LocalComputers, RemoteComputers};
use glave_ports::computer::{Computers, Environment};
use glave_ports::testing::{TempDir, read_program};
use glave_ports::{block_on, conformance};
use glave_process::Processes;

const GLAVE: &str = env!("CARGO_BIN_EXE_glave");

/// What a terminal's shell starts with: bash, at home in `home`, so the person's own profile
/// plays no part.
fn shell_in(home: &std::path::Path) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| name == "PATH")
        .chain([
            ("HOME".into(), home.as_os_str().to_owned()),
            ("SHELL".into(), "/bin/bash".into()),
        ])
        .collect()
}

#[test]
fn this_machine_opens_terminals_with_glave_as_their_starter_directly_and_through_the_gate() {
    let dir = TempDir::new("terminals-here").unwrap();
    let folder = dir.join("work");
    std::fs::create_dir_all(&folder).unwrap();
    read_program(GLAVE).unwrap();
    block_on!(async {
        let tied = Processes::unrecorded().with_lifeline(GLAVE).await.unwrap();
        for processes in [Processes::unrecorded(), tied] {
            let computers = LocalComputers::new(dir.join("copies"), shell_in(&dir))
                .with_helper(GLAVE)
                .with_processes(processes);
            let computer = computers.create(&Environment::host()).await.unwrap().id;
            let verdict =
                conformance::terminals(&computers, computer, &folder.to_string_lossy()).await;
            assert_eq!(verdict, Ok(()));
        }
    });
}

#[test]
fn the_binary_serves_terminals_across_the_protocol() {
    let dir = TempDir::new("terminals-served").unwrap();
    let folder = dir.join("work");
    std::fs::create_dir_all(&folder).unwrap();
    read_program(GLAVE).unwrap();
    block_on!(async {
        let mut child = tokio::process::Command::new(GLAVE)
            .args(["computer", "serve", "--stdio"])
            .env_clear()
            .envs(shell_in(&dir))
            .env("GLAVE_HOME", dir.join("home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdout.take().unwrap();
        let output = child.stdin.take().unwrap();
        let remote = RemoteComputers::connect(input, output);
        remote.check_protocol().await.unwrap();
        let computer = remote.create(&Environment::host()).await.unwrap().id;
        let verdict = conformance::terminals(&remote, computer, &folder.to_string_lossy()).await;
        assert_eq!(verdict, Ok(()));
        drop(remote);
        let status = child.wait().await.unwrap();
        assert!(status.success(), "the server ends cleanly with its input");
    });
}
