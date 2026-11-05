//! How a turn begins: its configuration stored, `turn.started`, and the person's message,
//! with what changed since the model was told before it (`prefix.rs`).

use glave_event::{
    BlobRef, MessageAdded, ModelRef, Part, Permissions, Principal, Role, TurnStarted,
};
use glave_ports::{BlobStore, EventLog, Message, ModelProvider, Pieces, ToolSpec};
use serde::Serialize;

use super::Prompt;
use crate::approval::Approver;
use crate::harness::{Harness, HarnessError, Thread};
use crate::memory::ConfigMemory;
use crate::sandbox::{Recorded, recorded};
use crate::toolset::ToolSet;

/// The version of the turn configuration blob.
const CONFIG_VERSION: u32 = 2;

/// The most a turn's configuration may be: its system prompt and tools, many times over.
pub(crate) const CONFIG_LIMIT: u64 = 16 * 1024 * 1024;

/// The full configuration of a turn, stored once by hash and referenced from `turn.started`.
#[derive(Serialize)]
struct TurnConfig<'a> {
    /// The blob's format: 2 since it records memory. Blobs without `v` are version 1.
    v: u32,
    model: &'a str,
    provider: &'a str,
    system: &'a str,
    tools: &'a [ToolSpec],
    max_output_tokens: Option<u32>,
    /// What the agent could do without asking: the thread's mode.
    permissions: Permissions,
    /// The sandbox commands ran in; absent when there was none.
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox: Option<Recorded>,
    /// The memories the model knew at the turn's start, at their versions; absent when the
    /// thread reads no memory, so a configuration without memory is what it always was.
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<ConfigMemory>,
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// Records what ended in the background since the last turn, stores the turn's
    /// configuration, then records `turn.started` and the person's message.
    pub(super) async fn begin(
        &self,
        thread: &mut Thread,
        prompt: Prompt<'_>,
        offer: &ToolSet,
    ) -> Result<(), HarnessError> {
        let Prompt {
            text: prompt,
            files,
            caller,
            by_glave,
            id,
        } = prompt;
        // Where it works now that its project's folder moved, before the turn that works there.
        if let Some(bound) = thread.rebound.take() {
            self.record(thread, &bound).await?;
            // Glave's own record: call ids do not count it, so the model is sent the same.
            thread.unnumbered = thread.unnumbered.saturating_add(1);
        }
        // What ended in the background since the last turn is recorded before this one.
        self.check_commands(thread).await?;
        let sent_to = self.model_for(thread);
        let provider = self.current_default().provider;
        let config = self
            .store_config(thread, offer, &sent_to, &provider)
            .await?;
        let model = ModelRef {
            provider,
            name: sent_to,
        };
        // A new thread opens with the project's instructions.
        let opening = thread.messages.is_empty();
        let mut parts = thread
            .opening_parts()
            .filter(|_| opening)
            .unwrap_or_default();
        // What changed since the model was told, before the person's words (`prefix.rs`).
        parts.extend(crate::prefix::news(offer));
        parts.push(Part::Text {
            text: prompt.to_owned(),
        });
        parts.extend_from_slice(files);
        let mut events = Vec::new();
        self.stage(
            thread,
            &mut events,
            &TurnStarted {
                model,
                config,
                trace: caller.trace.clone(),
                auth: caller.auth.clone(),
                missing: offer.missing.clone(),
            },
        )?;
        let message = MessageAdded {
            role: Role::User,
            parts: parts.clone(),
            author: if by_glave {
                Some(Principal::glave())
            } else {
                Self::other_than_for(thread, caller.person)
            },
            thought: None,
        };
        match id {
            Some(id) => self.stage_as(thread, &mut events, &message, id)?,
            None => self.stage(thread, &mut events, &message)?,
        }
        self.commit(thread, events).await?;
        crate::prefix::heard(thread, offer);
        thread.turns = thread.turns.saturating_add(1);
        thread.messages.push(Message::User { parts });
        if by_glave {
            return Ok(());
        }
        thread.request = Some(prompt.to_owned());
        if !thread.delegated() {
            thread.asked.push(prompt.to_owned());
        }
        Ok(())
    }

    /// Stores the turn's full configuration, as sent to `model` at `provider`, and answers
    /// the reference `turn.started` holds.
    async fn store_config(
        &self,
        thread: &mut Thread,
        offer: &ToolSet,
        model: &str,
        provider: &str,
    ) -> Result<BlobRef, HarnessError> {
        let memory = self.look_at_memory(thread).await;
        let config = TurnConfig {
            v: CONFIG_VERSION,
            model,
            provider,
            system: &offer.system,
            tools: &offer.specs,
            max_output_tokens: self.settings.max_output_tokens,
            permissions: self.permissions_for(thread),
            sandbox: self.workspace_for(thread).sandbox().map(recorded),
            memory,
        };
        let bytes = serde_json::to_vec(&config).map_err(|e| HarnessError::Event(e.to_string()))?;
        let stored = self.blobs.put(
            &thread.scope,
            Some(thread.id),
            "application/json",
            CONFIG_LIMIT,
            Box::new(Pieces::from(bytes)),
        );
        Ok(stored.await?)
    }
}
