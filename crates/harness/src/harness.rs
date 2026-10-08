//! The harness: its parts, its settings, and how it records events.

use std::collections::HashSet;

use glave_event::{
    At, EncodeError, EventId, Header, InvalidValue, Payload, Producer, RawEvent, Seq, ThreadId,
    ThreadStarted, encode,
};
use glave_ports::{
    BlobError, BlobStore, EventLog, LogError, Message, ModelProvider, ProviderError, Scope,
};
use thiserror::Error;

use crate::approval::Approver;
use crate::env::{Clock, Entropy};
use crate::prompt::system_prompt;
use crate::tools::{Tool, workspace::Workspace};

/// How the harness runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The model name sent to the provider.
    pub model: String,
    /// A short name for the provider, recorded with each turn, such as `openrouter`.
    pub provider: String,
    pub system_prompt: String,
    /// Model calls allowed in one turn before it stops.
    pub max_steps: u32,
    pub max_output_tokens: Option<u32>,
}

impl Settings {
    /// Settings for `model` at `provider`, with the default prompt naming that model.
    pub fn new(model: impl Into<String>, provider: impl Into<String>) -> Self {
        let model = model.into();
        let provider = provider.into();
        Self {
            system_prompt: system_prompt(&model, &provider),
            model,
            provider,
            max_steps: 50,
            max_output_tokens: None,
        }
    }
}

/// Everything the harness is wired to: the ports, the injected clock and randomness, and the
/// workspace the tools act on.
pub struct Parts<P, L, B, A> {
    pub provider: P,
    pub log: L,
    pub blobs: B,
    pub approver: A,
    pub clock: Box<dyn Clock>,
    pub entropy: Box<dyn Entropy>,
    pub workspace: Workspace,
}

/// The agent loop and everything it needs.
pub struct Harness<P, L, B, A> {
    pub(crate) provider: P,
    pub(crate) log: L,
    pub(crate) blobs: B,
    pub(crate) approver: A,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) entropy: Box<dyn Entropy>,
    pub(crate) workspace: Workspace,
    pub(crate) settings: Settings,
}

/// One thread: where it is stored, where it continues, and the conversation so far.
#[derive(Debug, Clone)]
pub struct Thread {
    pub scope: Scope,
    pub id: ThreadId,
    pub(crate) next_seq: Seq,
    pub(crate) messages: Vec<Message>,
    pub(crate) allowed: HashSet<Tool>,
}

impl Thread {
    /// The conversation the model will see next.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}

/// Why a turn could not run. Tool failures are not errors: the model sees them and adapts.
#[derive(Debug, Error)]
pub enum HarnessError {
    #[error(transparent)]
    Log(#[from] LogError),
    #[error(transparent)]
    Blob(#[from] BlobError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("could not build an event: {0}")]
    Event(String),
    #[error("thread {0} has no events in this space")]
    NotFound(ThreadId),
}

impl From<EncodeError> for HarnessError {
    fn from(error: EncodeError) -> Self {
        Self::Event(error.to_string())
    }
}

impl From<InvalidValue> for HarnessError {
    fn from(error: InvalidValue) -> Self {
        Self::Event(error.to_string())
    }
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    pub fn new(parts: Parts<P, L, B, A>, settings: Settings) -> Self {
        Self {
            provider: parts.provider,
            log: parts.log,
            blobs: parts.blobs,
            approver: parts.approver,
            clock: parts.clock,
            entropy: parts.entropy,
            workspace: parts.workspace,
            settings,
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// The log events are written to.
    pub fn log(&self) -> &L {
        &self.log
    }

    /// Starts a new thread in `scope` and records `thread.started`.
    pub async fn start_thread(&self, scope: Scope) -> Result<Thread, HarnessError> {
        let id = ThreadId::new(self.clock.now(), self.entropy.bytes())?;
        let mut thread = Thread {
            scope,
            id,
            next_seq: Seq::FIRST,
            messages: Vec::new(),
            allowed: HashSet::new(),
        };
        let mut events = Vec::new();
        let producer = Producer {
            name: "glave".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        };
        self.stage(
            &thread,
            &mut events,
            &ThreadStarted {
                producer,
                fork: None,
            },
        )?;
        self.commit(&mut thread, events).await?;
        Ok(thread)
    }

    /// The `seq` the next staged event will get.
    pub(crate) fn seq_after(thread: &Thread, staged: &[RawEvent]) -> Result<Seq, HarnessError> {
        let offset = u64::try_from(staged.len()).map_err(|e| HarnessError::Event(e.to_string()))?;
        Seq::new(thread.next_seq.get().saturating_add(offset)).map_err(HarnessError::from)
    }

    /// Encodes `payload` as the next event of a batch that has not been written yet.
    pub(crate) fn stage<T: Payload>(
        &self,
        thread: &Thread,
        events: &mut Vec<RawEvent>,
        payload: &T,
    ) -> Result<(), HarnessError> {
        let now = self.clock.now();
        let header = Header {
            id: EventId::new(now, self.entropy.bytes())?,
            thread: thread.id,
            seq: Self::seq_after(thread, events)?,
            at: At::from(now),
        };
        events.push(encode(header, payload)?);
        Ok(())
    }

    /// Writes a batch all-or-nothing, then moves the thread past it.
    pub(crate) async fn commit(
        &self,
        thread: &mut Thread,
        events: Vec<RawEvent>,
    ) -> Result<(), HarnessError> {
        if events.is_empty() {
            return Ok(());
        }
        let next = Self::seq_after(thread, &events)?;
        self.log.append(&thread.scope, &events).await?;
        thread.next_seq = next;
        Ok(())
    }
}
