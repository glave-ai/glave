//! The calls a person's computer answers as it is: each goes to the computer holding the
//! workspace. And, for the tests, an output that keeps the end of what a command printed.

use glave_ports::computer::{ComputerId, ComputerInfo, Size, State};

/// What a computer this adapter does not hold is: gone.
pub(crate) fn gone(computer: ComputerId) -> ComputerInfo {
    ComputerInfo {
        id: computer,
        state: State::Gone,
        os: "linux".into(),
        arch: String::new(),
        size: Size::default(),
        environment: None,
        disk: None,
        largest: Vec::new(),
    }
}

/// The most of what a command printed that a [`Tail`] keeps.
#[cfg(test)]
const KEPT: usize = 8_000;

/// Keeps the end of what a command printed, for a test to read.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Tail(std::sync::Mutex<String>);

#[cfg(test)]
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

#[cfg(test)]
impl glave_ports::computer::Output for Tail {
    fn write<'a>(&'a self, text: &'a str) -> glave_ports::BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
            // Bounded.
            let excess = all.len().saturating_sub(KEPT);
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

/// The [`glave_ports::computer::Computers`] methods that go straight to the computer holding
/// the workspace.
macro_rules! forward {
    () => {
        fn read<'a>(
            &'a self,
            workspace: WorkspaceId,
            path: &'a str,
        ) -> BoxFuture<'a, Result<glave_ports::computer::Bytes, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.read(workspace, path).await
            })
        }

        fn write<'a>(
            &'a self,
            workspace: WorkspaceId,
            files: &'a [glave_ports::computer::FileWrite],
        ) -> BoxFuture<'a, Result<(), ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.write(workspace, files).await
            })
        }

        fn list<'a>(
            &'a self,
            workspace: WorkspaceId,
            listing: &'a glave_ports::computer::Listing,
        ) -> BoxFuture<'a, Result<glave_ports::computer::Listed, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.list(workspace, listing).await
            })
        }

        fn stat<'a>(
            &'a self,
            workspace: WorkspaceId,
            path: &'a str,
        ) -> BoxFuture<'a, Result<glave_ports::computer::Stat, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.stat(workspace, path).await
            })
        }

        fn glob<'a>(
            &'a self,
            workspace: WorkspaceId,
            glob: &'a glave_ports::computer::Glob,
        ) -> BoxFuture<'a, Result<glave_ports::computer::Globbed, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.glob(workspace, glob).await
            })
        }

        fn search<'a>(
            &'a self,
            workspace: WorkspaceId,
            search: &'a glave_ports::computer::Search,
        ) -> BoxFuture<'a, Result<glave_ports::computer::Searched, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.search(workspace, search).await
            })
        }

        fn changes<'a>(
            &'a self,
            workspace: WorkspaceId,
            since: Option<&'a str>,
        ) -> BoxFuture<'a, Result<Option<glave_ports::computer::Changes>, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.changes(workspace, since).await
            })
        }

        fn save<'a>(
            &'a self,
            workspace: WorkspaceId,
            save: &'a glave_ports::computer::Save,
        ) -> BoxFuture<'a, Result<glave_ports::computer::Saved, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.save(workspace, save).await
            })
        }

        fn output(
            &self,
            workspace: WorkspaceId,
            command: glave_ports::computer::CommandId,
            wait: std::time::Duration,
        ) -> BoxFuture<'_, Result<glave_ports::computer::Printed, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker)
                    .await?
                    .output(workspace, command, wait)
                    .await
            })
        }

        fn ended(
            &self,
            workspace: WorkspaceId,
            command: glave_ports::computer::CommandId,
            wait: std::time::Duration,
        ) -> BoxFuture<'_, Result<Option<glave_ports::computer::Finished>, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker)
                    .await?
                    .ended(workspace, command, wait)
                    .await
            })
        }

        fn stop(
            &self,
            workspace: WorkspaceId,
            command: glave_ports::computer::CommandId,
        ) -> BoxFuture<'_, Result<glave_ports::computer::Printed, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.stop(workspace, command).await
            })
        }

        fn commands(
            &self,
            workspace: WorkspaceId,
        ) -> BoxFuture<'_, Result<Vec<glave_ports::computer::Background>, ComputerError>> {
            Box::pin(async move {
                let worker = self.worker_of(workspace)?;
                self.remote(&worker).await?.commands(workspace).await
            })
        }
    };
}

pub(crate) use forward;
