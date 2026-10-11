//! The calls a worker answers as it is: each goes to the worker holding the workspace. And
//! the small pieces setup needs: an output that keeps the end of what a command printed, and
//! a cancellation that never fires.

use std::sync::Mutex;

use glave_ports::{BoxFuture, Cancellation, ComputerId, ComputerInfo, Output, Size, State};

/// What a computer this adapter does not hold is: gone.
pub(crate) fn gone(computer: ComputerId) -> ComputerInfo {
    ComputerInfo {
        id: computer,
        state: State::Gone,
        os: "linux".into(),
        arch: String::new(),
        size: Size::default(),
        environment: None,
    }
}

/// Keeps what a command printed, for an error that says why it failed.
#[derive(Default)]
pub(crate) struct Tail(Mutex<String>);

impl Tail {
    /// The last `most` bytes, cut at a character boundary.
    pub(crate) fn text(&self, most: usize) -> String {
        let all = self.0.lock().map(|all| all.clone()).unwrap_or_default();
        let mut start = all.len().saturating_sub(most);
        while !all.is_char_boundary(start) {
            start = start.saturating_add(1);
        }
        all.get(start..).unwrap_or_default().trim().to_owned()
    }
}

impl Output for Tail {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
            // Bounded: keep twice what is ever shown.
            let excess = all.len().saturating_sub(2 * super::SETUP_TAIL);
            if excess > 0 {
                let mut cut = excess;
                while !all.is_char_boundary(cut) {
                    cut = cut.saturating_add(1);
                }
                all.replace_range(..cut, "");
            }
        }
        Box::pin(std::future::ready(()))
    }
}

/// A cancellation that never fires: setup runs to its end or its timeout.
pub(crate) struct Never;

impl Cancellation for Never {
    fn is_cancelled(&self) -> bool {
        false
    }

    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}

/// The [`glave_ports::Computers`] methods that go straight to the worker holding the
/// workspace. Writing is new work, so it waits on the worker's run time; reading, finding and
/// saving do not, so a worker out of time can still be saved.
macro_rules! forward {
    () => {
        fn read<'a>(
            &'a self,
            workspace: WorkspaceId,
            path: &'a str,
        ) -> BoxFuture<'a, Result<glave_ports::Bytes, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.read(workspace, path).await
            })
        }

        fn write<'a>(
            &'a self,
            workspace: WorkspaceId,
            files: &'a [glave_ports::FileWrite],
        ) -> BoxFuture<'a, Result<(), ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.check_time(&worker)?;
                self.remote(&worker).await?.write(workspace, files).await
            })
        }

        fn list<'a>(
            &'a self,
            workspace: WorkspaceId,
            listing: &'a glave_ports::Listing,
        ) -> BoxFuture<'a, Result<glave_ports::Listed, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.list(workspace, listing).await
            })
        }

        fn stat<'a>(
            &'a self,
            workspace: WorkspaceId,
            path: &'a str,
        ) -> BoxFuture<'a, Result<glave_ports::Stat, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.stat(workspace, path).await
            })
        }

        fn glob<'a>(
            &'a self,
            workspace: WorkspaceId,
            glob: &'a glave_ports::Glob,
        ) -> BoxFuture<'a, Result<glave_ports::Globbed, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.glob(workspace, glob).await
            })
        }

        fn search<'a>(
            &'a self,
            workspace: WorkspaceId,
            search: &'a glave_ports::Search,
        ) -> BoxFuture<'a, Result<glave_ports::Searched, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.search(workspace, search).await
            })
        }

        fn save<'a>(
            &'a self,
            workspace: WorkspaceId,
            save: &'a glave_ports::Save,
        ) -> BoxFuture<'a, Result<glave_ports::Saved, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.save(workspace, save).await
            })
        }
    };
}

pub(crate) use forward;
