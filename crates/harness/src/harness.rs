//! The harness: its parts, its settings, and how it records events.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use glave_event::{
    At, EncodeError, EventId, Header, InvalidValue, Origin, OriginKind, ParentCall, Part, Payload,
    Principal, PrincipalId, Producer, RawEvent, Seq, ThreadId, ThreadStarted, encode,
};
use glave_ports::{
    BlobError, BlobStore, EventLog, LogError, Message, ModelProvider, ProviderError, Scope,
    ToolSource,
};
use thiserror::Error;

use crate::approval::{ApprovalMode, Approver};
use crate::builtins::Builtins;
use crate::compaction::{Compaction, Measured};
use crate::env::{Clock, Entropy};
use crate::prompt::system_prompt;
use crate::retry::Retries;
use crate::tools::workspace::Workspace;
use crate::toolset::ToolSet;

/// How the harness runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The model name sent to the provider.
    pub model: String,
    /// A short name for the provider, recorded with each turn, such as `openrouter`.
    pub provider: String,
    pub system_prompt: String,
    /// The project's instructions (see [`crate::agents_md`]), sent at the start of every new
    /// thread and again after a compaction.
    pub instructions: Option<String>,
    /// How much to ask the person before acting.
    pub approval: ApprovalMode,
    /// When to summarize a thread to keep it within the model's context window.
    pub compaction: Compaction,
    /// How a model call that fails in a way that may pass is tried again.
    pub retries: Retries,
    /// Model calls allowed in one turn before it stops; no cap when `None`, as in Codex and
    /// OpenCode. A subagent's turn gets the same cap of its own.
    pub max_steps: Option<u32>,
    pub max_output_tokens: Option<u32>,
    /// Whether the model may hand work to subagents with `agent`.
    pub subagents: bool,
    /// Whether, in auto mode, a judge decides if an action a rule flagged may run because the
    /// person asked for it. Its model is chosen by the provider's `judge` route (see the
    /// gateway); off, every flagged action asks.
    pub judge: bool,
    /// The person whose work every thread is: recorded as the thread's `for`. When `None`,
    /// the harness makes one id when it is created, used for all its threads.
    pub person: Option<PrincipalId>,
    /// The agent profile that runs every thread: recorded as the thread's `agent`. When
    /// `None`, made once like `person`.
    pub agent: Option<PrincipalId>,
}

impl Settings {
    /// Settings for `model` at `provider`, with the default prompt naming that model.
    pub fn new(model: impl Into<String>, provider: impl Into<String>) -> Self {
        let model = model.into();
        let provider = provider.into();
        Self {
            system_prompt: system_prompt(&model, &provider),
            instructions: None,
            model,
            provider,
            approval: ApprovalMode::default(),
            compaction: Compaction::default(),
            retries: Retries::default(),
            max_steps: None,
            max_output_tokens: None,
            subagents: false,
            judge: false,
            person: None,
            agent: None,
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
/// Who a harness's threads work for, and the agent that runs them. A thread's subagents work
/// for the same person and, until agent profiles have their own ids, under the same agent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Actors {
    pub(crate) person: Option<PrincipalId>,
    pub(crate) agent: Option<PrincipalId>,
}

pub struct Harness<P, L, B, A> {
    pub(crate) provider: P,
    pub(crate) log: L,
    pub(crate) blobs: B,
    pub(crate) approver: A,
    pub(crate) clock: Box<dyn Clock>,
    pub(crate) entropy: Box<dyn Entropy>,
    pub(crate) workspace: Arc<Workspace>,
    /// Where tools come from: the built-in tools first, then any others, such as MCP servers.
    pub(crate) sources: Vec<Arc<dyn ToolSource>>,
    pub(crate) settings: Settings,
    /// Who every thread works for, and the agent that runs it: from the settings, or made once.
    pub(crate) actors: Actors,
    /// The model's real context window, once a refusal has stated it; 0 until then.
    pub(crate) learned_window: AtomicU64,
}

/// One thread: where it is stored, where it continues, and the conversation so far.
#[derive(Debug, Clone)]
pub struct Thread {
    pub scope: Scope,
    pub id: ThreadId,
    pub(crate) next_seq: Seq,
    pub(crate) messages: Vec<Message>,
    /// Tools the person allowed for the rest of the thread, by name.
    pub(crate) allowed: HashSet<String>,
    /// The size the provider last reported, for estimating the next request.
    pub(crate) measured: Option<Measured>,
    /// The person's latest request, kept word for word through a compaction.
    pub(crate) request: Option<String>,
    /// The person's own messages, oldest first: what the judge reads as their say-so. A
    /// subagent's thread holds its parent's, never the brief the parent's agent wrote.
    pub(crate) asked: Vec<String>,
    /// The threads that started this one, nearest first: empty unless this is a subagent's
    /// thread, whose prompts an agent, not the person, writes.
    pub(crate) above: Vec<ThreadId>,
}

impl Thread {
    /// The conversation the model will see next.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Whether an agent, not the person, writes this thread's prompts: a subagent's thread.
    pub(crate) fn delegated(&self) -> bool {
        !self.above.is_empty()
    }

    /// This thread, then the threads that started it: whose budgets its model calls count in.
    pub(crate) fn lineage(&self) -> Vec<ThreadId> {
        let mut threads = Vec::with_capacity(self.above.len().saturating_add(1));
        threads.push(self.id);
        threads.extend_from_slice(&self.above);
        threads
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
    #[error("cancelled")]
    Cancelled,
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
        let workspace = Arc::new(parts.workspace);
        let made = || PrincipalId::new(parts.clock.now(), parts.entropy.bytes()).ok();
        let actors = Actors {
            person: settings.person.or_else(made),
            agent: settings.agent.or_else(made),
        };
        Self {
            actors,
            provider: parts.provider,
            log: parts.log,
            blobs: parts.blobs,
            approver: parts.approver,
            clock: parts.clock,
            entropy: parts.entropy,
            sources: vec![Arc::new(Builtins {
                workspace: Arc::clone(&workspace),
            })],
            workspace,
            settings,
            learned_window: AtomicU64::new(0),
        }
    }

    /// Offers the tools of `sources` too, after the built-in ones. A tool keeps the first
    /// source that names it.
    #[must_use]
    pub fn with_tools(mut self, sources: Vec<Arc<dyn ToolSource>>) -> Self {
        self.sources.extend(sources);
        self
    }

    /// The tools on offer now, from every source.
    pub(crate) async fn tools(&self) -> ToolSet {
        let mut set = ToolSet::gather(&self.sources, &self.settings.system_prompt).await;
        if self.approver.can_answer() {
            set.offer_question();
        }
        if self.settings.subagents {
            set.offer_agent();
        }
        set
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// The log events are written to.
    pub fn log(&self) -> &L {
        &self.log
    }

    /// Who is asked before anything changes.
    pub fn approver(&self) -> &A {
        &self.approver
    }

    /// Starts a new thread in `scope` and records `thread.started`.
    pub async fn start_thread(&self, scope: Scope) -> Result<Thread, HarnessError> {
        self.start_thread_from(scope, None).await
    }

    /// Starts a thread, recording the call in another thread that started it, if one did.
    pub(crate) async fn start_thread_from(
        &self,
        scope: Scope,
        parent: Option<ParentCall>,
    ) -> Result<Thread, HarnessError> {
        let id = ThreadId::new(self.clock.now(), self.entropy.bytes())?;
        let mut thread = Thread {
            scope,
            id,
            next_seq: Seq::FIRST,
            messages: Vec::new(),
            allowed: HashSet::new(),
            measured: None,
            request: None,
            asked: Vec::new(),
            above: parent.iter().map(|call| call.thread).collect(),
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
                origin: Origin::of(if parent.is_some() {
                    OriginKind::Subagent
                } else {
                    OriginKind::Prompt
                }),
                producer,
                fork: None,
                parent,
                for_: self.actors.person.map(Principal::person),
                agent: self.actors.agent.map(Principal::agent),
            },
        )?;
        self.commit(&mut thread, events).await?;
        Ok(thread)
    }

    /// The project's instructions as the opening part of a conversation, if there are any.
    pub(crate) fn instruction_parts(&self) -> Option<Vec<Part>> {
        self.settings
            .instructions
            .as_ref()
            .map(|text| vec![Part::Text { text: text.clone() }])
    }

    /// Records one event on its own.
    pub(crate) async fn record<T: Payload>(
        &self,
        thread: &mut Thread,
        payload: &T,
    ) -> Result<(), HarnessError> {
        let mut events = Vec::new();
        self.stage(thread, &mut events, payload)?;
        self.commit(thread, events).await
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
