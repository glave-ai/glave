//! Which model a thread's turns are sent to, and what the agent may do in them: the ones last
//! chosen for it, recorded as `model.chosen` and `permissions.chosen`, else its project's, else
//! the harness's own. A subagent's thread uses its parent's.

use std::borrow::Cow;
use std::sync::PoisonError;

use glave_event::{Body, EffortChosen, Event, ModelChosen, Permissions, PermissionsChosen};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::approval::{ApprovalMode, Approver};
use crate::harness::{Harness, HarnessError, Thread};
use crate::prompt::system_prompt;

/// The longest model id, in bytes.
pub(crate) const MAX_MODEL_ID: usize = 255;

/// The longest reasoning level, in bytes.
const MAX_EFFORT: usize = 32;

/// The model the latest `model.chosen` in `events` names, if any.
pub(crate) fn chosen_in(events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|event| match &event.body {
        Body::ModelChosen(chosen) => Some(chosen.model.clone()),
        _ => None,
    })
}

/// The model threads that chose none are sent to, and its provider's short name.
#[derive(Debug, Clone)]
pub(crate) struct DefaultModel {
    pub(crate) model: String,
    pub(crate) provider: String,
}

impl Thread {
    /// The model chosen for this thread, when someone chose one: its turns go there instead
    /// of the harness's own model.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The mode chosen for this thread, when someone chose one: its turns run with it
    /// instead of its project's or the harness's own.
    pub fn permissions(&self) -> Option<Permissions> {
        self.permissions
    }

    /// How much this thread's model is asked to reason, when someone chose it.
    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref()
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The model `thread`'s turns are sent to: its own choice, else its project's, else the
    /// default now.
    pub(crate) fn model_for(&self, thread: &Thread) -> String {
        thread
            .model
            .clone()
            .or_else(|| thread.brought.settled.model.clone())
            .unwrap_or_else(|| self.default_model())
    }

    /// The model threads that chose none are sent to now: the settings' until the host
    /// changes it; empty while there is none.
    pub fn default_model(&self) -> String {
        self.current_default().model
    }

    /// Sends the turns of threads that chose no model to `model`, at the provider named
    /// `provider`, from their next turn on, as when the person connects a model account.
    pub fn set_default_model(&self, model: impl Into<String>, provider: impl Into<String>) {
        let mut default = self.default.write().unwrap_or_else(PoisonError::into_inner);
        *default = DefaultModel {
            model: model.into(),
            provider: provider.into(),
        };
    }

    /// Tells the harness which models read no images, nor PDFs (which models read as images
    /// of their pages), as the host's catalog says: each file goes to one by its name. Every
    /// other model is sent them inline.
    pub fn set_without_images(&self, models: Vec<String>) {
        *self
            .without_images
            .write()
            .unwrap_or_else(PoisonError::into_inner) = models;
    }

    /// Takes `model` for one that reads no images from now on, as a refusal showed, until the
    /// host says again which do.
    pub(crate) fn learn_without_images(&self, model: &str) {
        let mut blind = self
            .without_images
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if !blind.iter().any(|known| known == model) {
            blind.push(model.to_owned());
        }
    }

    /// Whether `model` reads images and PDFs: unless the host said it does not, or a refusal
    /// showed it.
    pub fn reads_images(&self, model: &str) -> bool {
        !self
            .without_images
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|blind| blind == model)
    }

    /// The default model now, and its provider's name.
    pub(crate) fn current_default(&self) -> DefaultModel {
        self.default
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The system prompt `thread`'s turns are sent: glave's default prompt names the model
    /// actually answering, so a thread with a model of its own gets the prompt naming it. A
    /// prompt someone wrote is sent as written.
    pub(crate) fn system_for(&self, thread: &Thread) -> Cow<'_, str> {
        let settings = &self.settings;
        let model = self.model_for(thread);
        if model != settings.model
            && settings.system_prompt == system_prompt(&settings.model, &settings.provider)
        {
            Cow::Owned(system_prompt(&model, &self.current_default().provider))
        } else {
            Cow::Borrowed(&settings.system_prompt)
        }
    }

    /// Sends `thread`'s turns to `model` from now on, recorded as `model.chosen` for
    /// `caller`. Choosing the model already chosen records nothing. An id that is empty or
    /// longer than 255 bytes is refused; whether the model may be used is the
    /// caller's to check.
    pub async fn choose_model(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        model: &str,
    ) -> Result<(), HarnessError> {
        let model = model.trim();
        if model.is_empty() || model.len() > MAX_MODEL_ID {
            return Err(HarnessError::Invalid(format!(
                "a model id is 1 to {MAX_MODEL_ID} bytes, not only spaces"
            )));
        }
        if thread.model.as_deref() == Some(model) {
            return Ok(());
        }
        let chosen = ModelChosen {
            model: model.to_owned(),
            by: Self::other_than_for(thread, caller.person),
        };
        self.record(thread, &chosen).await?;
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        thread.model = Some(model.to_owned());
        Ok(())
    }

    /// What the agent may do in `thread`: its own choice, else its project's, else the
    /// harness's mode, never above the policy's cap ([`crate::Defaults::permissions`]).
    pub fn permissions_for(&self, thread: &Thread) -> Permissions {
        let protections = thread.brought.settled.protections;
        thread.defaults.permissions(protections, thread.permissions)
    }

    /// How much `thread`'s turns ask before acting, as its mode comes to it where its
    /// commands run. Full access never asks. Ask first lets commands run in its read-only
    /// sandbox and asks before every change, as Codex's Read Only does; Auto asks only before
    /// risky actions behind a wall (the sandbox, or a worker's own container). Without a wall,
    /// both ask before every change and command. Plan comes to what Ask first does, and what
    /// would ask there is refused instead (`Harness::ask`).
    pub(crate) fn approval_for(&self, thread: &Thread) -> ApprovalMode {
        let workspace = self.workspace_for(thread);
        let sandboxed = workspace.sandbox().is_some();
        let worker = workspace.info().computer != self.workspace.info().computer;
        match self.permissions_for(thread) {
            Permissions::FullAccess => ApprovalMode::Never,
            Permissions::Plan | Permissions::Ask if sandboxed => ApprovalMode::Changes,
            Permissions::Auto if sandboxed || worker => ApprovalMode::Risky,
            Permissions::Plan | Permissions::Ask | Permissions::Auto => ApprovalMode::Everything,
        }
    }

    /// Whether `thread` runs in plan mode: it changes nothing until the person approves its
    /// plan.
    pub(crate) fn planning(&self, thread: &Thread) -> bool {
        self.permissions_for(thread) == Permissions::Plan
    }

    /// Runs `thread`'s turns with `permissions` from now on, recorded as `permissions.chosen`
    /// for `caller`. Choosing the mode already chosen records nothing. Whether the mode is
    /// allowed is the caller's to check, against the policy.
    pub async fn choose_permissions(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        permissions: Permissions,
    ) -> Result<(), HarnessError> {
        if thread.permissions == Some(permissions) {
            return Ok(());
        }
        let chosen = PermissionsChosen {
            permissions,
            by: Self::other_than_for(thread, caller.person),
        };
        self.record(thread, &chosen).await?;
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        thread.permissions = Some(permissions);
        Ok(())
    }

    /// Asks `thread`'s model to reason at `effort` from now on, recorded as `effort.chosen`
    /// for `caller`. Choosing the level already chosen records nothing. A level that is empty,
    /// longer than 32 bytes or not one plain word is refused; whether the model offers it is
    /// the caller's to check.
    pub async fn choose_effort(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        effort: &str,
    ) -> Result<(), HarnessError> {
        let effort = effort.trim();
        let plain = effort
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
        if effort.is_empty() || effort.len() > MAX_EFFORT || !plain {
            return Err(HarnessError::Invalid(format!(
                "a reasoning level is one word of 1 to {MAX_EFFORT} letters, such as high"
            )));
        }
        if thread.effort.as_deref() == Some(effort) {
            return Ok(());
        }
        let chosen = EffortChosen {
            effort: effort.to_owned(),
            by: Self::other_than_for(thread, caller.person),
        };
        self.record(thread, &chosen).await?;
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        thread.effort = Some(effort.to_owned());
        Ok(())
    }
}

/// The level the latest `effort.chosen` in `events` names, if any.
pub(crate) fn effort_in(events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|event| match &event.body {
        Body::EffortChosen(chosen) => Some(chosen.effort.clone()),
        _ => None,
    })
}

/// The mode the latest `permissions.chosen` in `events` names, if any.
pub(crate) fn permissions_in(events: &[Event]) -> Option<Permissions> {
    events.iter().rev().find_map(|event| match &event.body {
        Body::PermissionsChosen(chosen) => Some(chosen.permissions),
        _ => None,
    })
}
