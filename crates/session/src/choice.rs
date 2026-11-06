//! What a request chooses for a thread's turns, kept for the turns after it: the model, how
//! much it reasons, and what the agent may do (`specs/approvals.md`, "Modes"). A thread's
//! mode is its latest `permissions.chosen`, else its project's, else the server's own, never
//! above the policy's cap, and only a mode the cap leaves may be chosen, so the log tells any
//! server of the deployment what a thread may do.

use std::path::Path;

use glave_event::{Body, Permissions, PrincipalId, ThreadId};
use glave_harness::{HarnessError, Settled, Thread};
use glave_ports::computer::SandboxLevel;
use glave_ports::settings::Protections;
use glave_ports::{BlobStore, Caller, EventLog, Located, ModelProvider, Scope};

use crate::protocol::{ApiError, code};
use crate::server::{Glave, Server};
use crate::words::Sentence;

/// The type of the event that records a thread's mode.
const CHOSEN: &str = "permissions.chosen";

/// What `thread/start` or `turn/start` chose for the thread: each kept for its later turns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Choice {
    /// The model its turns go to.
    pub(crate) model: Option<String>,
    /// What the agent may do in its turns.
    pub(crate) permissions: Option<Permissions>,
    /// How much its model reasons: `Some(None)` is Default, the model's own again.
    #[expect(
        clippy::option_option,
        reason = "as the API's `effort`: left out keeps the thread's level, `null` is Default"
    )]
    pub(crate) effort: Option<Option<String>>,
}

/// The modes a thread may choose on a server whose own is `own`, under the cap `highest`, in the
/// order a person is offered them, and always `own`.
pub(crate) fn choosable(own: Permissions, highest: Permissions) -> Vec<Permissions> {
    Permissions::ALL
        .into_iter()
        .filter(|mode| *mode == own || *mode <= highest)
        .collect()
}

/// A mode as the person reads it in the server's sentences.
pub(crate) fn label(mode: Permissions) -> &'static str {
    match mode {
        Permissions::Auto => "Auto",
        Permissions::Plan => "Plan",
        Permissions::Ask => "Ask first",
        Permissions::FullAccess => "Full access",
    }
}

/// `names` as a person reads a choice among them: "Auto, Plan or Ask first".
fn either(names: &[&str]) -> String {
    match names.split_last() {
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
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
    if let Some(mode) = choice.permissions {
        harness.choose_permissions(thread, caller, mode).await?;
    }
    if let Some(effort) = &choice.effort {
        harness
            .choose_effort(thread, caller, effort.as_deref())
            .await?;
    }
    Ok(())
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Refuses `mode` with `-32012` unless a thread of `person` here may choose it: Full
    /// access, which has no sandbox, only where they may leave it.
    pub(crate) fn allowed_permissions(
        &self,
        person: Option<PrincipalId>,
        mode: Permissions,
    ) -> Result<(), ApiError> {
        let modes = self.modes(person);
        if modes.contains(&mode) {
            return Ok(());
        }
        let left: Vec<&str> = modes.into_iter().map(label).collect();
        let left = either(&left);
        let sentence = if self.modes(None).contains(&mode) {
            Sentence::FullAccessOwnerOnly { left }
        } else {
            Sentence::PermissionsNotAllowed {
                mode: label(mode).to_owned(),
                left,
            }
        };
        Err(ApiError::new(code::NOT_ALLOWED, sentence))
    }

    /// The mode a thread of `person` runs with now, as its summary shows it, as its turns run
    /// with it: its own choice, else its project's, else the server's, as `project` changes
    /// them, and Auto at most where they may not leave the sandbox.
    pub(crate) async fn permissions_now(
        &self,
        scope: &Scope,
        thread: ThreadId,
        person: Option<PrincipalId>,
        project: Protections,
    ) -> Result<Permissions, ApiError> {
        let chosen = self.chosen_permissions(scope, thread).await?;
        let mode = self.inner.harness.defaults().permissions(project, chosen);
        Ok(self.within_sandbox(person, mode))
    }

    /// The modes a thread of `person` may choose now, in the order a person is offered them:
    /// what the harness's cap leaves, without Full access where they may not leave the
    /// sandbox. One the thread chose before a cap came runs capped, never refused.
    pub(crate) fn modes(&self, person: Option<PrincipalId>) -> Vec<Permissions> {
        let defaults = self.inner.harness.defaults();
        let mut modes = choosable(defaults.permissions, defaults.highest);
        if !self.computer_is_theirs(person) {
            modes.retain(|mode| *mode != Permissions::FullAccess);
        }
        modes
    }

    /// `mode` as a thread of `person` runs with it here: Auto at most where they may not
    /// leave the sandbox, as the harness runs it.
    pub(crate) fn within_sandbox(
        &self,
        person: Option<PrincipalId>,
        mode: Permissions,
    ) -> Permissions {
        if self.computer_is_theirs(person) {
            mode
        } else {
            mode.min(Permissions::Auto)
        }
    }

    /// Why Auto asks before every change and command here for `person`, when it does: no
    /// sandbox can run on this computer (and Ask first asks so too), or only one that keeps
    /// files as they are. For someone whose computer it is not, why fewer of their commands
    /// run instead.
    pub(crate) fn unsandboxed(&self, person: Option<PrincipalId>) -> Option<Sentence> {
        let defaults = self.inner.harness.defaults();
        if self.inner.homes.is_some() {
            return defaults
                .home_sandbox
                .is_none()
                .then_some(Sentence::OwnComputerWall);
        }
        let theirs = self.computer_is_theirs(person);
        match defaults.sandbox.as_ref() {
            None if !theirs => Some(Sentence::UnsandboxedOwnerOnly),
            Some(sandbox) if sandbox.level != SandboxLevel::Project && !theirs => {
                Some(Sentence::OnlyReadOnlySandboxTheirs)
            }
            None => Some(Sentence::Unsandboxed),
            Some(sandbox) if sandbox.level != SandboxLevel::Project => {
                Some(Sentence::OnlyReadOnlySandbox)
            }
            Some(_) => None,
        }
    }

    /// What the project at `at` chooses for its threads: read from its folder here, for the
    /// person on this computer's threads; for a folder on its person's own computer, from what
    /// was last read there, which reaches nothing.
    pub(crate) async fn project_settled(&self, at: Located) -> Settled {
        let harness = &self.inner.harness;
        match self.folder_of(at).await {
            Ok(Some(folder)) if folder.checkout.computer == self.computer() => {
                harness.settled(Path::new(&folder.checkout.path)).await
            }
            Ok(Some(folder)) if self.inner.homes.is_some() => {
                let scope = Scope::glaves_own(at.org, at.project);
                let root = Path::new(&folder.checkout.path);
                harness.settled_at_home(scope, root).await
            }
            _ => Settled::default(),
        }
    }

    /// Whether the computer a thread of `person` works on is theirs: the person on this
    /// computer, the server's; or anyone where people have computers of their own. Then what
    /// a project chooses counts for it (on this computer once the person trusts it; on their
    /// own, what loosens once they do), and its commands may leave the sandbox. Anyone else's
    /// run on the server's computer, beside its secrets, so only in the sandbox
    /// (`specs/sandbox.md`, "Whose computer").
    pub(crate) fn computer_is_theirs(&self, person: Option<PrincipalId>) -> bool {
        self.inner.homes.is_some() || person.is_none_or(|person| Some(person) == self.person())
    }

    /// The mode last chosen for `thread`, from its log.
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
    use Permissions::{Ask, Auto, FullAccess, Plan};

    #[test]
    fn a_thread_may_choose_what_the_cap_leaves_and_the_servers_own_as_offered() {
        assert_eq!(choosable(Auto, FullAccess), [Auto, Plan, Ask, FullAccess]);
        assert_eq!(choosable(Auto, Auto), [Auto, Plan, Ask]);
        assert_eq!(choosable(Ask, Ask), [Plan, Ask]);
        assert_eq!(choosable(FullAccess, Ask), [Plan, Ask, FullAccess]);
    }

    #[test]
    fn a_choice_reads_as_a_person_says_it() {
        assert_eq!(
            either(&["Auto", "Plan", "Ask first"]),
            "Auto, Plan or Ask first"
        );
        assert_eq!(either(&["Plan", "Ask first"]), "Plan or Ask first");
        assert_eq!(either(&["Plan"]), "Plan");
    }

    #[test]
    fn each_mode_reads_as_the_picker_names_it() {
        let labels: Vec<&str> = Permissions::ALL.into_iter().map(label).collect();
        assert_eq!(labels, ["Auto", "Plan", "Ask first", "Full access"]);
    }
}
