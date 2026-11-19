//! The workspace an attempt works in: in the sandbox where it can run.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use glave_computer::LocalComputers;
use glave_harness::Workspace;
use glave_ports::computer::{Sandbox, SandboxCaches, SandboxLevel, SandboxNetwork};

use super::Setup;
use crate::error::EvalError;
use crate::prepare::Environment;
use crate::task::Task;

/// The workspace in `root`, in the sandbox, which the task's mode narrows, with `hidden`
/// hidden from it too. As glave does: where the sandbox cannot run, commands run without it,
/// and Auto and Ask first ask (with no one to ask, they are not run); a task that tests the
/// sandbox never gives way.
pub(super) async fn workspace(
    task: &Task,
    setup: &Setup,
    (environment, hidden): (&Environment, &[PathBuf]),
    root: &Path,
) -> Result<Workspace, EvalError> {
    let open = || async {
        let computers = LocalComputers::for_here(environment.clone())
            .with_processes(setup.processes.clone())
            .with_hidden(hidden.iter().cloned());
        let computers = match &setup.glave_on_path {
            Some(folder) => computers.with_glave_on_path(folder),
            None => computers,
        };
        let (computers, info) = computers
            .open_here(root)
            .await
            .map_err(|error| EvalError::io("open", root)(std::io::Error::other(error)))?;
        Ok::<_, EvalError>(Workspace::new(computers, info))
    };
    let sandbox = Sandbox {
        level: SandboxLevel::Project,
        network: SandboxNetwork::when(task.network),
        home: environment.get(&OsString::from("HOME")).map(PathBuf::from),
        glave_home: None,
        caches: SandboxCaches::Kept,
    };
    match open().await?.with_sandbox(&sandbox).await {
        Ok(workspace) => Ok(workspace),
        Err(_) if !task.sandboxed => open().await,
        Err(error) => Err(EvalError::io("sandbox", root)(std::io::Error::other(error))),
    }
}
