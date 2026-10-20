//! Where commands run on this computer (`specs/sandbox.md`): the workspace, in the sandbox the
//! choices ask for, and the choices that follow where glave's default sandbox cannot run. A
//! server that reads the person's settings again sandboxes its next turns the same way.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use glave_computer::LocalComputers;
use glave_config::policy::{self, Choices, Constraints, On, Source};
use glave_harness::{ApprovalMode, Workspace};
use glave_ports::computer::Sandbox;
use glave_process::Processes;

use crate::setup::Setup;
use crate::{home, identity};

/// The sandbox `choices` ask for, if any: the person's home, and glave's folder
/// (`glave_home`), protected in it.
pub(crate) fn sandbox_of(choices: &Choices, glave_home: Option<&Path>) -> Option<Sandbox> {
    choices.sandbox.value.map(|level| Sandbox {
        level,
        network: choices.network.value,
        home: std::env::var_os("HOME").map(PathBuf::from),
        glave_home: glave_home.map(Path::to_path_buf),
    })
}

/// `choices` once glave's default sandbox turned out not to run here, for the reason `why`:
/// approval becomes `ask` (`policy::unsandboxed`), unless threads work on workers, whose own
/// container or virtual machine is their wall.
pub(crate) fn given_up(choices: Choices, why: &str, constraints: &Constraints) -> Choices {
    let approval = choices.approval.clone();
    let mut choices = policy::unsandboxed(choices, why, constraints);
    if choices.on.value == On::Cloud {
        choices.approval = approval;
    }
    choices
}

/// The workspace in `folder`, in the sandbox the setup's choices ask for. Where glave's
/// default sandbox cannot run, commands go unconfined and the choices say so
/// ([`given_up`]). A sandbox someone chose, or a policy requires, is never given up: glave
/// refuses to start instead.
pub(crate) async fn workspace(
    folder: &Path,
    setup: &mut Setup,
    database: &Path,
    processes: &Processes,
) -> anyhow::Result<Workspace> {
    let choices = &setup.choices;
    let Some(sandbox) = sandbox_of(choices, database.parent()) else {
        return open_here(folder, processes).await;
    };
    let error = match open_here(folder, processes)
        .await?
        .with_sandbox(&sandbox)
        .await
    {
        Ok(workspace) => return Ok(workspace),
        Err(error) => error,
    };
    if choices.sandbox.from != Source::Default {
        return Err(error).with_context(|| {
            format!(
                "the sandbox was asked for (by {}), so glave will not run commands without it",
                choices.sandbox.from
            )
        });
    }
    let why = error.to_string();
    setup.choices = given_up(setup.choices.clone(), &why, &setup.loaded.constraints);
    if setup.choices.on.value != On::Cloud {
        eprintln!("glave: {}", unsandboxed_notice(&setup.choices, &why));
    }
    open_here(folder, processes).await
}

/// The workspace in `folder` on this computer, without a sandbox yet.
async fn open_here(folder: &Path, processes: &Processes) -> anyhow::Result<Workspace> {
    let computer = identity::computer(&home::folder()?).await?;
    let computers = LocalComputers::for_here(std::env::vars_os())
        .known_as(computer)
        .with_worktrees(home::folder()?.join(crate::worktrees::FOLDER))
        .with_scratch(home::folder()?.join(crate::worktrees::SCRATCH))
        .with_projects(home::projects()?)
        .with_processes(processes.clone());
    let (computers, here) = crate::computer::with_sandbox_helper(computers)
        .open_here(folder)
        .await?;
    Ok(Workspace::new(computers, here))
}

/// What the person is told, once at startup, when glave's default sandbox cannot run here.
fn unsandboxed_notice(choices: &Choices, why: &str) -> String {
    let asks = if choices.approval.value == ApprovalMode::Ask {
        "so every command and change asks you first (approval: ask)"
    } else {
        "and the policy allows only approval auto, so commands run as you without asking"
    };
    format!(
        "the sandbox cannot run on this computer ({why}), {asks}. To run commands without \
         the sandbox and without asking, choose both sandbox = \"off\" and approval = \"auto\" \
         (--sandbox off --approval auto)."
    )
}
