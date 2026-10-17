//! What a request chooses for a thread's turns, kept for the turns after it: the model, and
//! how much they ask before acting (`specs/approvals.md`, "Modes"). A thread's approval mode is
//! its latest `approval.mode.chosen`, else the server's own, and only a mode the policy leaves
//! may be chosen, so the log tells any server of the deployment which mode a thread runs in.

use glave_event::{Body, ThreadId};
use glave_harness::{Caller, HarnessError, Thread};
use glave_ports::{BlobStore, EventLog, ModelProvider, Scope, SpaceId};

use crate::protocol::{ApiError, ApprovalMode, code};
use crate::server::{Glave, Server};

/// The type of the event that records a thread's approval mode.
const CHOSEN: &str = "approval.mode.chosen";

/// What `thread/start` or `turn/start` chose for the thread: each kept for its later turns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Choice {
    /// The model its turns go to.
    pub(crate) model: Option<String>,
    /// How much its turns ask before acting.
    pub(crate) approval: Option<ApprovalMode>,
}

/// The modes a thread may choose on a server whose own mode is `own`: `allowed`, cautious
/// first, and always `own`.
pub(crate) fn choosable(own: ApprovalMode, allowed: &[ApprovalMode]) -> Vec<ApprovalMode> {
    [ApprovalMode::Ask, ApprovalMode::Auto]
        .into_iter()
        .filter(|mode| *mode == own || allowed.contains(mode))
        .collect()
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
    if let Some(mode) = choice.approval {
        harness.choose_approval(thread, caller, mode).await?;
    }
    Ok(())
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Refuses `mode` with `-32012` unless a thread here may choose it.
    pub(crate) fn allowed_mode(&self, mode: ApprovalMode) -> Result<(), ApiError> {
        if self.inner.modes.contains(&mode) {
            return Ok(());
        }
        let left: Vec<&str> = self.inner.modes.iter().map(|mode| mode.name()).collect();
        Err(ApiError::new(
            code::NOT_ALLOWED,
            format!(
                "approval mode {} is not allowed here: the policy leaves {}",
                mode.name(),
                left.join(" and ")
            ),
        ))
    }

    /// Refuses a turn in `thread` that would run in a mode no longer allowed here: `asked`,
    /// else the mode it chose before. A thread that chose none runs in the server's own.
    pub(crate) async fn check_approval(
        &self,
        thread: ThreadId,
        asked: Option<ApprovalMode>,
    ) -> Result<(), ApiError> {
        let mode = if asked.is_some() {
            asked
        } else {
            let scope = self.scope_of(thread).await?;
            self.chosen_approval(&scope, thread).await?
        };
        match mode {
            Some(mode) => self.allowed_mode(mode),
            None => Ok(()),
        }
    }

    /// The mode `thread` runs in now, as its summary shows it: its own choice, else its
    /// project's (`project`), else the server's.
    pub(crate) async fn approval_now(
        &self,
        scope: &Scope,
        thread: ThreadId,
        project: Option<ApprovalMode>,
    ) -> Result<ApprovalMode, ApiError> {
        Ok(self
            .chosen_approval(scope, thread)
            .await?
            .or(project)
            .unwrap_or(self.inner.harness.settings().approval))
    }

    /// The mode the project of `space` gives its threads that chose none, when its trusted
    /// settings loosen it.
    pub(crate) async fn project_approval(&self, space: SpaceId) -> Option<ApprovalMode> {
        let folder = self.folder_here(space).await.ok().flatten()?;
        let loosened = self
            .inner
            .harness
            .loosened(std::path::Path::new(&folder))
            .await;
        loosened.approval
    }

    /// The mode last chosen for `thread`, from its log.
    async fn chosen_approval(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> Result<Option<ApprovalMode>, ApiError> {
        Ok(match self.latest(scope, thread, CHOSEN).await? {
            Some(Body::ApprovalModeChosen(chosen)) => Some(chosen.mode),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_harness::ApprovalMode as Mode;

    #[test]
    fn a_thread_may_choose_what_the_policy_leaves_and_the_servers_own() {
        assert_eq!(
            choosable(ApprovalMode::Auto, &[Mode::Ask, Mode::Auto]),
            [ApprovalMode::Ask, ApprovalMode::Auto]
        );
        assert_eq!(
            choosable(ApprovalMode::Ask, &[Mode::Ask]),
            [ApprovalMode::Ask]
        );
        assert_eq!(choosable(ApprovalMode::Auto, &[]), [ApprovalMode::Auto]);
    }
}
