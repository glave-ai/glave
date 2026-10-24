//! Where commands run on this computer (`specs/sandbox.md`): the workspace, in the most a
//! sandbox here allows, which each thread's preset narrows, and the choices that follow where
//! none can run. A server that reads the person's settings again sandboxes its next turns the
//! same way.

use std::path::{Path, PathBuf};

use glave_computer::LocalComputers;
use glave_config::policy::{Choices, On};
use glave_harness::{Permissions, Workspace};
use glave_ports::computer::{Sandbox, SandboxLevel};
use glave_process::Processes;

use crate::setup::Setup;
use crate::{home, identity};

/// The sandbox at `level`, as `choices` ask: with the network as chosen, the person's home and
/// glave's folder (`glave_home`) protected in it.
pub(crate) fn sandbox_at(
    level: SandboxLevel,
    choices: &Choices,
    glave_home: Option<&Path>,
) -> Sandbox {
    Sandbox {
        level,
        network: choices.network.value,
        home: std::env::var_os("HOME").map(PathBuf::from),
        glave_home: glave_home.map(Path::to_path_buf),
    }
}

/// The workspace in `folder`, in the most a sandbox here allows: the project; else only
/// reading, where only that can run (Landlock without bubblewrap); its commands asking for the
/// network unless the policy forbids it. Where the project's sandbox cannot run, Workspace asks
/// as Read-only does and the choices say why, said once here; a policy that requires the
/// sandbox refuses to start instead.
pub(crate) async fn workspace(
    folder: &Path,
    setup: &mut Setup,
    processes: &Processes,
) -> anyhow::Result<Workspace> {
    let asks = setup.loaded.constraints.network_ask_forbidden().is_none();
    let database = home::database().await?;
    let glave_home = database.parent();
    let project = sandbox_at(SandboxLevel::Project, &setup.choices, glave_home);
    let why = match open_here(folder, processes, asks)
        .await?
        .with_sandbox(&project)
        .await
    {
        Ok(workspace) => {
            setup.sandbox = Some(project);
            return Ok(workspace);
        }
        Err(error) => error.to_string(),
    };
    if let Some(rule) = setup.loaded.constraints.sandbox_required() {
        anyhow::bail!(
            "the sandbox cannot run on this computer ({why}), and {rule} requires it, so glave \
             will not run commands without it"
        );
    }
    setup.choices.unsandboxed = Some(why.clone());
    let quiet = setup.choices.on.value == On::Cloud
        || setup.choices.permissions.value == Permissions::FullAccess;
    if !quiet {
        eprintln!("glave: {}", unsandboxed_notice(&why));
    }
    let reading = sandbox_at(SandboxLevel::ReadOnly, &setup.choices, glave_home);
    match open_here(folder, processes, asks)
        .await?
        .with_sandbox(&reading)
        .await
    {
        Ok(workspace) => {
            setup.sandbox = Some(reading);
            Ok(workspace)
        }
        Err(_) => open_here(folder, processes, asks).await,
    }
}

/// The workspace in `folder` on this computer, without a sandbox yet, its commands asking for
/// the network when they have none if `asks`.
async fn open_here(folder: &Path, processes: &Processes, asks: bool) -> anyhow::Result<Workspace> {
    let computer = identity::computer(&home::folder()?).await?;
    let computers = LocalComputers::for_here(std::env::vars_os())
        .known_as(computer)
        .with_worktrees(home::folder()?.join(crate::worktrees::FOLDER))
        .with_scratch(home::folder()?.join(crate::worktrees::SCRATCH))
        .with_projects(home::projects()?)
        .with_processes(processes.clone());
    let (computers, here) = crate::helpers::with_sandbox_helper(computers)
        .open_here(folder)
        .await?;
    Ok(Workspace::new(computers, here).asking_network(asks))
}

/// What the person is told, once at startup, when the sandbox cannot run here.
fn unsandboxed_notice(why: &str) -> String {
    format!(
        "the sandbox cannot run on this computer ({why}), so every command and change asks \
         you first. To let commands run without the sandbox and without asking, choose full \
         access (permissions = \"full-access\", or --permissions full-access)."
    )
}
