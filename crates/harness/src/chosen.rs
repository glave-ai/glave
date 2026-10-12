//! Which model a thread's turns are sent to, and how much they ask before acting: the ones
//! last chosen for it, recorded as `model.chosen` and `approval.mode.chosen`, else the
//! harness's own. A subagent's thread uses its parent's.

use std::borrow::Cow;

use glave_event::{ApprovalMode, ApprovalModeChosen, Body, Event, ModelChosen};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::prompt::system_prompt;

/// The longest model id, in bytes.
pub(crate) const MAX_MODEL_ID: usize = 255;

/// The model the latest `model.chosen` in `events` names, if any.
pub(crate) fn chosen_in(events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|event| match &event.body {
        Body::ModelChosen(chosen) => Some(chosen.model.clone()),
        _ => None,
    })
}

impl Thread {
    /// The model chosen for this thread, when someone chose one: its turns go there instead
    /// of the harness's own model.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The approval mode chosen for this thread, when someone chose one: its turns run in it
    /// instead of the harness's own.
    pub fn approval(&self) -> Option<ApprovalMode> {
        self.approval
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// The model `thread`'s turns are sent to.
    pub fn model_for<'a>(&'a self, thread: &'a Thread) -> &'a str {
        thread.model.as_deref().unwrap_or(&self.settings.model)
    }

    /// The system prompt `thread`'s turns are sent: glave's default prompt names the model
    /// actually answering, so a thread with a model of its own gets the prompt naming it. A
    /// prompt someone wrote is sent as written.
    pub(crate) fn system_for(&self, thread: &Thread) -> Cow<'_, str> {
        let settings = &self.settings;
        match thread.model.as_deref() {
            Some(model)
                if model != settings.model
                    && settings.system_prompt
                        == system_prompt(&settings.model, &settings.provider) =>
            {
                Cow::Owned(system_prompt(model, &settings.provider))
            }
            _ => Cow::Borrowed(&settings.system_prompt),
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

    /// How much `thread`'s turns ask before acting: its own choice, else the harness's mode.
    pub fn approval_for(&self, thread: &Thread) -> ApprovalMode {
        thread.approval.unwrap_or(self.settings.approval)
    }

    /// Runs `thread`'s turns in approval mode `mode` from now on, recorded as
    /// `approval.mode.chosen` for `caller`. Choosing the mode already chosen records nothing.
    /// Whether the mode is allowed is the caller's to check, against the policy.
    pub async fn choose_approval(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        mode: ApprovalMode,
    ) -> Result<(), HarnessError> {
        if thread.approval == Some(mode) {
            return Ok(());
        }
        let chosen = ApprovalModeChosen {
            mode,
            by: Self::other_than_for(thread, caller.person),
        };
        self.record(thread, &chosen).await?;
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        thread.approval = Some(mode);
        Ok(())
    }
}

/// The approval mode the latest `approval.mode.chosen` in `events` names, if any.
pub(crate) fn approval_in(events: &[Event]) -> Option<ApprovalMode> {
    events.iter().rev().find_map(|event| match &event.body {
        Body::ApprovalModeChosen(chosen) => Some(chosen.mode),
        _ => None,
    })
}
