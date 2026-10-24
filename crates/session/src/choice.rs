//! What a request chooses for a thread's turns, kept for the turns after it: the model, how
//! much it reasons, and what the agent may do (`specs/approvals.md`, "Permissions"). A thread's
//! preset is its latest `permissions.chosen`, else its project's, else the server's own, and
//! only a preset the policy leaves may be chosen, so the log tells any server of the deployment
//! what a thread may do.

use glave_event::{Body, Permissions, ThreadId};
use glave_harness::{HarnessError, Settled, Thread};
use glave_ports::settings::Protections;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider, ProjectId, Scope};

use crate::protocol::{ApiError, code};
use crate::server::{Glave, Server};
use crate::words::Sentence;

/// The type of the event that records a thread's preset.
const CHOSEN: &str = "permissions.chosen";

/// What `thread/start` or `turn/start` chose for the thread: each kept for its later turns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Choice {
    /// The model its turns go to.
    pub(crate) model: Option<String>,
    /// What the agent may do in its turns.
    pub(crate) permissions: Option<Permissions>,
    /// How much its model reasons.
    pub(crate) effort: Option<String>,
}

/// The presets a thread may choose on a server whose own is `own`: `allowed`, the most cautious
/// first, and always `own`.
pub(crate) fn choosable(own: Permissions, allowed: &[Permissions]) -> Vec<Permissions> {
    Permissions::ALL
        .into_iter()
        .filter(|preset| *preset == own || allowed.contains(preset))
        .collect()
}

/// A preset as the person reads it in the server's sentences.
pub(crate) fn label(preset: Permissions) -> &'static str {
    match preset {
        Permissions::ReadOnly => "Read-only",
        Permissions::Workspace => "Workspace",
        Permissions::FullAccess => "Full access",
    }
}

/// Records what `choice` chooses in `thread` for `caller`: what the thread already has records
/// nothing.
pub(crate) async fn record<P, L, B>(
    harness: &Glave<P, L, B>,
    thread: &mut Thread,
    caller: &Caller,
    choice: &Choice,
) -> Result<(), HarnessError>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    if let Some(model) = &choice.model {
        harness.choose_model(thread, caller, model).await?;
    }
    if let Some(preset) = choice.permissions {
        harness.choose_permissions(thread, caller, preset).await?;
    }
    if let Some(effort) = &choice.effort {
        harness.choose_effort(thread, caller, effort).await?;
    }
    Ok(())
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Refuses `preset` with `-32012` unless a thread here may choose it.
    pub(crate) fn allowed_permissions(&self, preset: Permissions) -> Result<(), ApiError> {
        let presets = self.presets();
        if presets.contains(&preset) {
            return Ok(());
        }
        let left: Vec<&str> = presets.into_iter().map(label).collect();
        Err(ApiError::new(
            code::NOT_ALLOWED,
            Sentence::PermissionsNotAllowed {
                preset: label(preset).to_owned(),
                left: left.join(" or "),
            },
        ))
    }

    /// Refuses a turn in `thread` that would run with a preset no longer allowed here: `asked`,
    /// else the one it chose before. A thread that chose none runs with the server's own.
    pub(crate) async fn check_permissions(
        &self,
        thread: ThreadId,
        asked: Option<Permissions>,
    ) -> Result<(), ApiError> {
        let preset = if asked.is_some() {
            asked
        } else {
            let scope = self.scope_of(thread).await?;
            self.chosen_permissions(&scope, thread).await?
        };
        match preset {
            Some(preset) => self.allowed_permissions(preset),
            None => Ok(()),
        }
    }

    /// The preset `thread` runs with now, as its summary shows it, as its turns run with it:
    /// its own choice, else its project's, else the server's, as `project` changes them.
    pub(crate) async fn permissions_now(
        &self,
        scope: &Scope,
        thread: ThreadId,
        project: Protections,
    ) -> Result<Permissions, ApiError> {
        let chosen = self.chosen_permissions(scope, thread).await?;
        Ok(project.permissions(chosen, self.inner.harness.defaults().permissions))
    }

    /// The presets a thread may choose now, the most cautious first.
    pub(crate) fn presets(&self) -> Vec<Permissions> {
        self.inner
            .presets
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Why Read-only and Workspace ask before every change and command here, when they do:
    /// no sandbox can run on this computer.
    pub(crate) fn unsandboxed(&self) -> Option<Sentence> {
        self.inner
            .harness
            .defaults()
            .sandbox
            .is_none()
            .then_some(Sentence::Unsandboxed)
    }

    /// What the project of `project` chooses for its threads: for the person on this
    /// computer's threads.
    pub(crate) async fn project_settled(&self, project: ProjectId) -> Settled {
        match self.folder_here(project).await {
            Ok(Some(folder)) => {
                let folder = std::path::Path::new(&folder);
                self.inner.harness.settled(folder).await
            }
            _ => Settled::default(),
        }
    }

    /// The preset last chosen for `thread`, from its log.
    async fn chosen_permissions(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<Permissions>, ApiError> {
        Ok(match self.latest(scope, thread, CHOSEN).await? {
            Some(Body::PermissionsChosen(chosen)) => Some(chosen.permissions),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Permissions::{FullAccess, ReadOnly, Workspace};

    #[test]
    fn a_thread_may_choose_what_the_policy_leaves_and_the_servers_own() {
        assert_eq!(choosable(Workspace, &Permissions::ALL), Permissions::ALL);
        assert_eq!(
            choosable(Workspace, &[ReadOnly, Workspace]),
            [ReadOnly, Workspace]
        );
        assert_eq!(choosable(FullAccess, &[]), [FullAccess]);
    }
}
