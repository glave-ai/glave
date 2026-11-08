//! What a terminal on this machine refuses before any shell starts. The terminals suite, which
//! needs glave's own program to start a shell, runs on this machine and across the wire in the
//! binary's tests (`crates/cli/tests/computer.rs`).

use std::time::Duration;

use glave_ports::block_on;
use glave_ports::computer::{
    Bytes, ComputerError, Computers, Environment, TerminalId, TerminalSize,
};
use glave_ports::testing::TempDir;

use crate::LocalComputers;

const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };

#[test]
fn without_glave_s_own_program_no_terminal_opens_and_none_is_found() {
    let dir = TempDir::new("terminal-refused").unwrap();
    let computers = LocalComputers::new(dir.join("copies"), Vec::new());
    block_on!(async {
        let computer = computers.create(&Environment::host()).await.unwrap().id;
        let folder = dir.to_string_lossy().into_owned();
        let opened = computers.open_terminal(computer, &folder, SIZE).await;
        assert!(
            matches!(opened, Err(ComputerError::Unsupported(_))),
            "{opened:?}"
        );
        let flat = TerminalSize { cols: 80, rows: 0 };
        let opened = computers.open_terminal(computer, &folder, flat).await;
        assert!(matches!(opened, Err(ComputerError::Refused(_))), "{opened:?}");
        let nowhere = format!("{folder}/not-there");
        let opened = computers.open_terminal(computer, &nowhere, SIZE).await;
        assert!(matches!(opened, Err(ComputerError::NotFound(_))), "{opened:?}");

        let stray = TerminalId::from_uuid(uuid::Uuid::nil());
        let input = Bytes::new(b"ls\r".to_vec());
        let not_found = |result: Result<(), ComputerError>| {
            matches!(result, Err(ComputerError::NotFound(_)))
        };
        assert!(not_found(computers.write_terminal(computer, stray, &input).await));
        assert!(not_found(computers.resize_terminal(computer, stray, SIZE).await));
        assert!(not_found(computers.close_terminal(computer, stray).await));
        let read = computers
            .read_terminal(computer, stray, 0, Duration::ZERO)
            .await;
        assert!(matches!(read, Err(ComputerError::NotFound(_))), "{read:?}");
    });
}
