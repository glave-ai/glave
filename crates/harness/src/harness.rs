//! The harness: its parts, its settings, and how it records events.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use glave_event::{
    At, EncodeError, EventId, Header, InvalidValue, Origin, OriginKind, ParentCall, Part, Payload,
    Principal, PrincipalId, Producer, RawEvent, Seq, ThreadId, ThreadStarted, encode,
};
use glave_ports::{
    BlobError, BlobStore, ComputerError, EventLog, LogError, Message, ModelProvider, ProviderError,
    Scope, ToolSource,
};
use thiserror::Error;

use crate::approval::{ApprovalMode, Approver};
use crate::bind::Place;
use crate::builtins::Builtins;
use crate::caller::Caller;
use crate::commands::Command;
use crate::compaction::{Compaction, Measured};
use crate::env::{Clock, Entropy};
use crate::goal::{GoalProgress, Pursuit};
use crate::hooks::Hooks;
use crate::prompt::system_prompt;
use crate::retry::Retries;
use crate::standing::Standing;
use crate::tools::workspace::Workspace;
use crate::toolset::ToolSet;
use crate::worker::Workers;

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
    /// The commands the person can type as `/name arguments`: a prompt that names one is
    /// sent as its body, with the arguments put in. None by default.
    pub commands: Vec<Command>,
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
            commands: Vec::new(),
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
    /// Commands run at points of a turn; none by default.
    pub(crate) hooks: Hooks,
    pub(crate) settings: Settings,
    /// Who every thread works for, and the agent that runs it: from the settings, or made once.
    pub(crate) actors: Actors,
    /// The model's real context window, once a refusal has stated it; 0 until then.
    pub(crate) learned_window: AtomicU64,
    /// Where threads placed on a worker get one; none by default.
    pub(crate) workers: Option<Workers>,
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
    /// The thread's latest goal, active or ended; `None` until one is set.
    pub(crate) goal: Option<Pursuit>,
    /// The workspace the thread's tools act on when it is not the harness's own, such as its
    /// worktree. A subagent's thread shares its parent's.
    pub(crate) workspace: Option<Arc<Workspace>>,
    /// Events glave recorded for itself that call ids do not count (`bind::own_records`).
    /// Each takes a `seq` before any later call, so ids stay unique.
    pub(crate) unnumbered: u64,
    /// What the model is told about where it works when that is not the folder itself: a
    /// worker's sources, and how work leaves it. Sent with the opening instructions.
    pub(crate) note: Option<String>,
    /// Whether the thread is shown, archived or deleted.
    pub(crate) standing: Standing,
    /// A cleanup removed the thread's worktree while it was held: it is opened again before
    /// the thread's next turn (`outside.rs`).
    pub(crate) removed: bool,
    /// Whether the thread has read untrusted content, such as a web page: risky actions after
    /// it are judged knowing that (`untrusted.rs`). A subagent's thread starts with its
    /// parent's.
    pub(crate) untrusted: bool,
}

impl Thread {
    /// The conversation the model will see next.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// The thread's latest goal and how far it has come, active or ended.
    pub fn goal(&self) -> Option<&GoalProgress> {
        self.goal.as_ref().map(|goal| &goal.progress)
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
    /// The thread was archived, from outside this harness, while it held the thread.
    #[error("thread {0} is archived; unarchive it first")]
    Archived(ThreadId),
    #[error("cancelled")]
    Cancelled,
    #[error("thread {0} has no active goal")]
    NoGoal(ThreadId),
    /// The thread's workspace could not be opened, such as a worktree outside a repository.
    #[error("{}", .0.message())]
    Workspace(#[from] ComputerError),
    /// A `UserPromptSubmit` hook stopped the prompt; the model never saw it.
    #[error("a hook stopped this prompt: {0}")]
    PromptBlocked(String),
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
            hooks: Hooks::default(),
            settings,
            learned_window: AtomicU64::new(0),
            workers: None,
        }
    }

    /// Places threads asked to work on a worker ([`crate::Place::Worker`]) with `workers`.
    #[must_use]
    pub fn with_workers(mut self, workers: Workers) -> Self {
        self.workers = Some(workers);
        self
    }

    /// Offers the tools of `sources` too, after the built-in ones. A tool keeps the first
    /// source that names it.
    #[must_use]
    pub fn with_tools(mut self, sources: Vec<Arc<dyn ToolSource>>) -> Self {
        self.sources.extend(sources);
        self
    }

    /// Runs `hooks` at their points of every turn.
    #[must_use]
    pub fn with_hooks(mut self, hooks: Hooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// The tools on offer now in `thread`, from every source.
    pub(crate) async fn tools(&self, thread: ThreadId) -> ToolSet {
        let mut set = ToolSet::gather(&self.sources, &self.settings.system_prompt).await;
        if self.approver.can_answer(thread) {
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
        self.start_thread_from(scope, None, &Caller::default(), None)
            .await
    }

    /// Starts a new thread for `caller`, recorded as the thread's `origin.by` when they are
    /// not the person it is for.
    pub async fn start_thread_as(
        &self,
        scope: Scope,
        caller: &Caller,
    ) -> Result<Thread, HarnessError> {
        self.start_thread_from(scope, None, caller, None).await
    }

    /// Starts a thread, recording the call in another thread that started it, if one did, and
    /// where it works, when `place` is given.
    pub(crate) async fn start_thread_from(
        &self,
        scope: Scope,
        parent: Option<ParentCall>,
        caller: &Caller,
        place: Option<Place>,
    ) -> Result<Thread, HarnessError> {
        let id = ThreadId::new(self.clock.now(), self.entropy.bytes())?;
        let placed = match place {
            Some(place) => Some(self.place(scope, id, place).await?),
            None => None,
        };
        let (workspace, bound) =
            placed.map_or((None, None), |(workspace, bound)| (workspace, Some(bound)));
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
            goal: None,
            note: bound.as_ref().and_then(crate::worker::note),
            workspace,
            unnumbered: u64::from(bound.is_some()),
            standing: Standing::Active,
            removed: false,
            untrusted: false,
        };
        let mut events = Vec::new();
        let mut origin = Origin::of(if parent.is_some() {
            OriginKind::Subagent
        } else {
            OriginKind::Prompt
        });
        origin.by = self.other_than_for(caller.person);
        let producer = Producer {
            name: "glave".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        };
        self.stage(
            &thread,
            &mut events,
            &ThreadStarted {
                origin,
                producer,
                fork: None,
                parent,
                for_: self.actors.person.map(Principal::person),
                agent: self.actors.agent.map(Principal::agent),
            },
        )?;
        if let Some(bound) = &bound {
            self.stage(&thread, &mut events, bound)?;
        }
        self.commit(&mut thread, events).await?;
        Ok(thread)
    }

    /// What opens a conversation: the project's instructions, and where the thread works when
    /// that is a worker, if there is either.
    pub(crate) fn opening_parts(&self, thread: &Thread) -> Option<Vec<Part>> {
        let parts: Vec<Part> = [self.settings.instructions.as_ref(), thread.note.as_ref()]
            .into_iter()
            .flatten()
            .map(|text| Part::Text { text: text.clone() })
            .collect();
        (!parts.is_empty()).then_some(parts)
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

    /// Writes a batch all-or-nothing, then moves the thread past it. When glave's own records
    /// were written to the thread from outside meanwhile, the batch follows them (`outside.rs`).
    pub(crate) async fn commit(
        &self,
        thread: &mut Thread,
        mut events: Vec<RawEvent>,
    ) -> Result<(), HarnessError> {
        if events.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.log.append(&thread.scope, &events).await {
            self.follow(thread, error, &mut events).await?;
            self.log.append(&thread.scope, &events).await?;
        }
        thread.next_seq = Self::seq_after(thread, &events)?;
        Ok(())
    }
}
