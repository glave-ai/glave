//! What this crate's tests share: a process that stands for one a glave left running, and how
//! many entries a ledger holds.

use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::sync::mpsc;

use rustix::process::{Pid, Signal, kill_process_group};

use crate::start::{self, Start};

/// A process left by a glave that is gone: `sleep` leading a group of its own, reaped the
/// moment it exits (as the system's init reaps a dead glave's children), its exit sent on.
pub(crate) struct Orphan {
    pub(crate) pid: i32,
    exited: mpsc::Receiver<()>,
}

impl Orphan {
    pub(crate) fn start() -> Self {
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let (sender, exited) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = child.wait();
            let _ = sender.send(());
        });
        Self { pid, exited }
    }

    pub(crate) fn start_time(&self) -> Start {
        start::of(&[self.pid]).remove(&self.pid).unwrap()
    }

    pub(crate) fn alive(&self) -> bool {
        self.exited.try_recv().is_err()
    }

    pub(crate) fn ended(&self) -> bool {
        self.exited.recv_timeout(glave_ports::testing::HUNG).is_ok()
    }
}

impl Drop for Orphan {
    fn drop(&mut self) {
        let _ = kill_process_group(Pid::from_raw(self.pid).unwrap(), Signal::KILL);
    }
}

/// How many entries the ledger in `folder` holds.
pub(crate) fn entries(folder: &Path) -> usize {
    std::fs::read_dir(folder).map_or(0, |listed| {
        listed
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .count()
    })
}
