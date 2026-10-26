//! What the person does with a proposed plan: edits it, or builds it. Building records the
//! approval, runs the thread in Auto from then on (or the highest mode the policy leaves, if
//! lower), and starts a turn at once whose prompt, glave's, is the plan as approved.

use glave_event::{Permissions, PlanApproved, PlanStatus, PlanUpdated, TurnResult};
use glave_ports::{BlobStore, Caller, EventLog, Message, ModelProvider};

use super::{checked, listed, told};
use crate::approval::Approver;
use crate::cancel::Cancel;
use crate::harness::{Harness, HarnessError, Thread};
use crate::turn::{Live, Prompt};

/// The prompt of the turn that builds an approved plan, before its steps.
const BUILD: &str = "The person approved this plan. Build it now, step by step, keeping it \
     current with update_plan: mark each step in_progress as you start it and completed once \
     it is done. If a step turns out wrong, change the plan and say why.";

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Replaces the steps of `thread`'s proposed plan with `steps`, as `caller` edited them,
    /// each to do: recorded as `plan.updated`, which the model is told of as a note of glave's
    /// before what comes next. Refused when no plan waits for approval, and for steps a plan
    /// cannot hold.
    pub async fn edit_plan(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        steps: &[String],
    ) -> Result<(), HarnessError> {
        if thread.plan.proposed().is_none() {
            return Err(HarnessError::NoPlan(thread.id));
        }
        let steps = checked(steps.iter().map(|step| (step.clone(), PlanStatus::Pending)))
            .map_err(HarnessError::Invalid)?;
        let edited = PlanUpdated {
            steps,
            call_id: None,
            proposed: true,
            by: Self::other_than_for(thread, caller.person),
        };
        self.record(thread, &edited).await?;
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        thread.plan.recorded(&edited);
        if let Some(parts) = told(&edited) {
            thread.messages.push(Message::User { parts });
        }
        Ok(())
    }

    /// The mode a plan is built in here: Auto, or the highest the policy leaves if lower;
    /// `None` where it leaves only Plan, so nothing can be built.
    pub fn builds_in(&self) -> Option<Permissions> {
        let mode = Permissions::Auto.min(self.defaults().highest);
        (mode != Permissions::Plan).then_some(mode)
    }

    /// Builds `thread`'s proposed plan for `caller`: records the approval (`plan.approved`),
    /// runs the thread in the mode [`Harness::builds_in`] gives from now on (recorded as
    /// `permissions.chosen` when it changes), and runs the turn that builds the plan as
    /// approved, its prompt glave's. Refused when no plan waits for approval, or where nothing
    /// can be built.
    pub async fn build_plan(
        &self,
        thread: &mut Thread,
        caller: &Caller,
        cancel: &Cancel,
        live: &mut (dyn FnMut(Live) + Send),
    ) -> Result<TurnResult, HarnessError> {
        self.close_left_open(thread).await?;
        let Some(plan) = thread.plan.proposed().cloned() else {
            return Err(HarnessError::NoPlan(thread.id));
        };
        let Some(mode) = self.builds_in() else {
            return Err(HarnessError::Invalid(
                "only Plan is allowed here, so no plan can be built".to_owned(),
            ));
        };
        let approved = PlanApproved {
            by: Self::other_than_for(thread, caller.person),
        };
        self.record(thread, &approved).await?;
        // Glave's own record: call ids do not count it, so the model is sent the same.
        thread.unnumbered = thread.unnumbered.saturating_add(1);
        thread.plan.approved();
        self.choose_permissions(thread, caller, mode).await?;
        let text = format!("{BUILD}\n\n<plan>\n{}</plan>", listed(&plan.steps));
        let prompt = Prompt {
            by_glave: true,
            ..Prompt::new(&text, caller)
        };
        self.run_prompt(thread, prompt, cancel, live).await
    }
}
