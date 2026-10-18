//! A computer that keeps a ledger of everything it did, for fault tests: each command and
//! each write is counted, so a test can tell whether anything ran twice. Commands named
//! `slow ...` take a minute of (paused) time, so a fault can land while one runs. It does no
//! real I/O, so tests can run on paused time.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use glave_ports::computer::{
    Bytes, Command, ComputerError, ComputerId, ComputerInfo, Computers, Ending, Environment,
    FileWrite, Finished, Glob, Globbed, Listed, Listing, Output, Save, Saved, Search, Searched,
    Size, Sources, Stat, State, WorkspaceId, WorkspaceInfo,
};
use glave_ports::{BoxFuture, Cancellation};

/// How long a `slow` command takes.
pub const SLOW: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Ledger {
    /// How many times each command and each write (`path=content`) was done.
    done: Mutex<HashMap<String, usize>>,
    files: Mutex<HashMap<String, Bytes>>,
}

impl Ledger {
    /// How many times `what` was done: a command's script, or `path=content` for a write.
    pub fn times(&self, what: &str) -> usize {
        self.done.lock().unwrap().get(what).copied().unwrap_or(0)
    }

    /// Puts a file in place without counting it.
    pub fn seed(&self, path: &str, content: &str) {
        self.files
            .lock()
            .unwrap()
            .insert(path.into(), Bytes::new(content.as_bytes()));
    }

    fn count(&self, what: String) {
        *self.done.lock().unwrap().entry(what).or_default() += 1;
    }
}

pub fn computer() -> ComputerId {
    ComputerId::from_uuid(uuid::Uuid::from_bytes([7; 16]))
}

pub fn workspace() -> WorkspaceId {
    WorkspaceId::from_uuid(uuid::Uuid::from_bytes([8; 16]))
}

fn unsupported<'a, T: Send + 'a>() -> BoxFuture<'a, Result<T, ComputerError>> {
    Box::pin(std::future::ready(Err(ComputerError::Unsupported(
        "not in the ledger".into(),
    ))))
}

impl Computers for Ledger {
    fn create<'a>(
        &'a self,
        _: &'a Environment,
    ) -> BoxFuture<'a, Result<ComputerInfo, ComputerError>> {
        unsupported()
    }

    fn status(&self, computer: ComputerId) -> BoxFuture<'_, Result<ComputerInfo, ComputerError>> {
        Box::pin(std::future::ready(Ok(ComputerInfo {
            id: computer,
            state: State::Running,
            os: "linux".into(),
            arch: "x86_64".into(),
            size: Size::default(),
            environment: None,
        })))
    }

    fn destroy(&self, _: ComputerId) -> BoxFuture<'_, Result<(), ComputerError>> {
        unsupported()
    }

    fn open<'a>(
        &'a self,
        computer: ComputerId,
        sources: &'a Sources,
    ) -> BoxFuture<'a, Result<WorkspaceInfo, ComputerError>> {
        Box::pin(std::future::ready(Ok(WorkspaceInfo {
            id: workspace(),
            computer,
            root: "/work".into(),
            sources: sources.clone(),
            home: None,
            temp: None,
        })))
    }

    fn close(&self, _: WorkspaceId) -> BoxFuture<'_, Result<(), ComputerError>> {
        unsupported()
    }

    fn run<'a>(
        &'a self,
        _: WorkspaceId,
        command: &'a Command,
        output: &'a dyn Output,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Result<Finished, ComputerError>> {
        Box::pin(async move {
            self.count(command.script.clone());
            output.write(&format!("ran {}\n", command.script)).await;
            let ending = if command.script.starts_with("slow") {
                tokio::select! {
                    () = tokio::time::sleep(SLOW) => Ending::Exited { code: 0 },
                    () = cancel.cancelled() => Ending::Cancelled,
                }
            } else {
                Ending::Exited { code: 0 }
            };
            Ok(Finished { ending, millis: 0 })
        })
    }

    fn read<'a>(
        &'a self,
        _: WorkspaceId,
        path: &'a str,
    ) -> BoxFuture<'a, Result<Bytes, ComputerError>> {
        let found = self.files.lock().unwrap().get(path).cloned();
        Box::pin(std::future::ready(found.ok_or_else(|| {
            ComputerError::NotFound(format!("{path}: no such file"))
        })))
    }

    fn write<'a>(
        &'a self,
        _: WorkspaceId,
        files: &'a [FileWrite],
    ) -> BoxFuture<'a, Result<(), ComputerError>> {
        for file in files {
            let text = String::from_utf8_lossy(file.content.as_slice()).into_owned();
            self.count(format!("{}={text}", file.path));
            self.files
                .lock()
                .unwrap()
                .insert(file.path.clone(), file.content.clone());
        }
        Box::pin(std::future::ready(Ok(())))
    }

    fn list<'a>(
        &'a self,
        _: WorkspaceId,
        _: &'a Listing,
    ) -> BoxFuture<'a, Result<Listed, ComputerError>> {
        unsupported()
    }

    fn stat<'a>(
        &'a self,
        _: WorkspaceId,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Stat, ComputerError>> {
        unsupported()
    }

    fn glob<'a>(
        &'a self,
        _: WorkspaceId,
        _: &'a Glob,
    ) -> BoxFuture<'a, Result<Globbed, ComputerError>> {
        unsupported()
    }

    fn search<'a>(
        &'a self,
        _: WorkspaceId,
        _: &'a Search,
    ) -> BoxFuture<'a, Result<Searched, ComputerError>> {
        unsupported()
    }

    fn save<'a>(
        &'a self,
        _: WorkspaceId,
        save: &'a Save,
    ) -> BoxFuture<'a, Result<Saved, ComputerError>> {
        self.count(format!("save {}", save.branch));
        Box::pin(std::future::ready(Ok(Saved {
            commit: "0".repeat(40),
            pushed: true,
        })))
    }
}
