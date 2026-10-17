//! The harness: its parts, its settings, and how it records events.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, RwLock};

use glave_event::{
    ApprovalMode, At, EncodeError, EventId, Header, InvalidValue, Origin, OriginKind, ParentCall,
    Part, Payload, Principal, PrincipalId, Producer, RawEvent, Seq, ThreadId, ThreadStarted,
    encode,
};
use glave_ports::memories::Memory;
use glave_ports::{
    BlobError, BlobStore, Caller, ComputerError, EventLog, LogError, Message, ModelProvider,
    ProviderError, Scope, ToolSource,
};
use thiserror::Error;

use crate::approval::Approver;
use crate::background::Commands;
use crate::bind::Place;
use crate::builtins::Builtins;
use crate::chosen::DefaultModel;
use crate::commands::Command;
use crate::compaction::{Compaction, Measured};
use crate::goal::{GoalProgress, Pursuit};
use crate::hooks::Hooks;
use crate::inbox::Inbox;
use crate::memory::MemoryState;
use crate::projects::{Brought, Projects};
use crate::prompt::system_prompt;
use crate::retry::Retries;
use crate::standing::Standing;
use crate::tools::workspace::Workspace;
use crate::toolset::ToolSet;
use crate::worker::Workers;
use glave_ports::{Clock, Entropy};

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
    /// Whether a model titles each thread after its first exchange (`title.rs`); off, a
    /// thread's title is the first line of its first message. Off by default.
    pub titles: bool,
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
            titles: false,
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
    /// The model threads that chose none go to: the settings' at first, then the host's.
    pub(crate) default: RwLock<DefaultModel>,
    /// Who every thread works for, and the agent that runs it: from the settings, or made once.
    pub(crate) actors: Actors,
    /// The model's real context window, once a refusal has stated it; 0 until then.
    pub(crate) learned_window: AtomicU64,
    /// Where threads placed on a worker get one; none by default.
    pub(crate) workers: Option<Workers>,
    /// What threads remember from one to the next; none by default, so the model is sent
    /// exactly what it is sent without memory.
    pub(crate) memory: Option<Arc<dyn Memory>>,
    /// Where threads in a folder of their own read their opening instructions, and what their
    /// project turns on; none by default, so every thread opens with the settings' own and
    /// gets only the harness's tools and hooks.
    pub(crate) projects: Option<Arc<dyn Projects>>,
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
    /// Whether the model has been shown a file that usually holds secrets: like untrusted
    /// content, it makes a save to memory a caution.
    pub(crate) read_secret: bool,
    /// What the model is shown of memory (`memory.rs`); empty when memory is off.
    pub(crate) memory: MemoryState,
    /// The person whose work this thread is, its `for`: the caller who started it, or the
    /// harness's person. A subagent's thread has its parent's.
    pub(crate) for_: Option<PrincipalId>,
    /// The model chosen for the thread (`chosen.rs`); the harness's own when `None`.
    pub(crate) model: Option<String>,
    /// The approval mode chosen for the thread (`chosen.rs`); the harness's own when `None`.
    pub(crate) approval: Option<ApprovalMode>,
    /// The instructions the thread opens with, and the judge reads as the project's rules:
    /// its folder's, or the settings' own (`bind.rs`). A subagent's thread has its parent's.
    pub(crate) instructions: Option<String>,
    /// The folder of the project the thread works in, when it works in one here: its own
    /// folder, or the one its worktree was made from. A subagent's thread has its parent's.
    pub(crate) project: Option<PathBuf>,
    /// What its project turned on at the start of the latest turn (`projects.rs`). A
    /// subagent's thread has its parent's.
    pub(crate) brought: Arc<Brought>,
    /// What the person sent its running turn, waiting for the model's next call (`inbox.rs`).
    pub(crate) inbox: Inbox,
    /// Its commands in the background (`background.rs`). A subagent's thread has its own.
    pub(crate) commands: Arc<Commands>,
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

    /// The person whose work this thread is: its recorded `for`.
    pub fn for_person(&self) -> Option<PrincipalId> {
        self.for_
    }

    /// Its commands in the background, which outlive its turns: shared, so they can be listed
    /// and stopped while a turn holds the thread.
    pub fn commands(&self) -> Arc<Commands> {
        Arc::clone(&self.commands)
    }

    /// What opens the conversation: the thread's instructions, and where it works when that
    /// is a worker, if there is either.
    pub(crate) fn opening_parts(&self) -> Option<Vec<Part>> {
        let parts: Vec<Part> = [self.instructions.as_ref(), self.note.as_ref()]
            .into_iter()
            .flatten()
            .map(|text| Part::Text { text: text.clone() })
            .collect();
        (!parts.is_empty()).then_some(parts)
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
    /// What was asked for is not valid, such as an empty title.
    #[error("{0}")]
    Invalid(String),
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
                commands: Arc::default(),
            })],
            workspace,
            hooks: Hooks::default(),
            default: RwLock::new(DefaultModel {
                model: settings.model.clone(),
                provider: settings.provider.clone(),
            }),
            settings,
            learned_window: AtomicU64::new(0),
            workers: None,
            memory: None,
            projects: None,
        }
    }

    /// Opens each thread that works in a folder of its own with what `projects` reads for that
    /// folder, rather than the settings' instructions, and gives it what its project turns on
    /// at the start of each turn.
    #[must_use]
    pub fn with_projects(mut self, projects: Option<Arc<dyn Projects>>) -> Self {
        self.projects = projects;
        self
    }

    /// Gives threads `memory`: loaded at their start, its tools offered, the person's controls
    /// served (`specs/memory.md`).
    #[must_use]
    pub fn with_memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Gives threads `memory` when there is one; none leaves them without.
    #[must_use]
    pub fn with_memory_if(mut self, memory: Option<Arc<dyn Memory>>) -> Self {
        self.memory = memory;
        self
    }

    /// The person threads work for unless their caller is someone else: from the settings, or
    /// made once.
    pub fn person(&self) -> Option<PrincipalId> {
        self.actors.person
    }

    /// The memory threads use, if any.
    pub fn memory(&self) -> Option<&Arc<dyn Memory>> {
        self.memory.as_ref()
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

    /// The tools on offer now in `thread`, from every source: the harness's, then its
    /// project's.
    pub(crate) async fn tools(&self, thread: &Thread) -> ToolSet {
        let sources = [self.sources.as_slice(), thread.brought.tools.as_slice()].concat();
        let mut set = ToolSet::gather(&sources, &self.system_for(thread)).await;
        if let Some(credentials) = self.workspace_for(thread).credentials()
            && let Some(note) = crate::prompt::signed_in(&credentials.hosts().await)
        {
            set.system = format!("{}\n\n{note}", set.system);
        }
        if let Some((memory, _)) = self.memory_for(thread)
            && let Ok(tools) = memory.tools().await
        {
            set.offer_memory(tools);
        }
        if self.approver.can_answer(thread.id) {
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

    /// What the harness was set up with: the default model at the start and the approval
    /// mode among them. The default model now is [`Harness::default_model`].
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The clock events are stamped with.
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// Where the random part of new ids comes from.
    pub fn entropy(&self) -> &dyn Entropy {
        self.entropy.as_ref()
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
        let for_ = caller.person.or(self.actors.person);
        let instructions = self
            .instructions_for(workspace.as_deref(), bound.as_ref(), for_)
            .await;
        let project = bound
            .as_ref()
            .and_then(|bound| crate::projects::project_of(&bound.opened));
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
            read_secret: false,
            memory: MemoryState::default(),
            for_,
            model: None,
            approval: None,
            instructions,
            project,
            brought: Arc::default(),
            inbox: Inbox::default(),
            commands: Arc::default(),
        };
        let mut events = Vec::new();
        let mut origin = Origin::of(if parent.is_some() {
            OriginKind::Subagent
        } else {
            OriginKind::Prompt
        });
        origin.by = Self::other_than_for(&thread, caller.person);
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
                for_: thread.for_.map(Principal::person),
                agent: self.actors.agent.map(Principal::agent),
            },
        )?;
        if let Some(bound) = &bound {
            self.stage(&thread, &mut events, bound)?;
        }
        self.commit(&mut thread, events).await?;
        Ok(thread)
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
        let id = EventId::new(now, self.entropy.bytes())?;
        Self::stage_at(thread, events, payload, id, now)
    }

    /// Like [`Harness::stage`], under an id made already: a message the person sent while a
    /// turn ran keeps the id they were answered with.
    pub(crate) fn stage_as<T: Payload>(
        &self,
        thread: &Thread,
        events: &mut Vec<RawEvent>,
        payload: &T,
        id: EventId,
    ) -> Result<(), HarnessError> {
        Self::stage_at(thread, events, payload, id, self.clock.now())
    }

    fn stage_at<T: Payload>(
        thread: &Thread,
        events: &mut Vec<RawEvent>,
        payload: &T,
        id: EventId,
        now: jiff::Timestamp,
    ) -> Result<(), HarnessError> {
        let header = Header {
            id,
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
