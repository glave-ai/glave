//! The harness: its parts, the threads it holds, and how it records events.

mod error;
mod settings;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use glave_event::{
    At, ComputerKind, EventId, Header, Origin, OriginKind, ParentCall, Part, Payload, Permissions,
    Principal, PrincipalId, Producer, RawEvent, Seq, ThreadId, ThreadStarted, WorkspaceBound,
    encode,
};
use glave_ports::computer::{ComputerId, WorkspaceId};
use glave_ports::memories::Memory;
use glave_ports::settings::Hooks;
use glave_ports::{BlobStore, Caller, EventLog, Message, ModelProvider, Scope, ToolSource};
use glave_ports::{Clock, Entropy, Payer, Standing};
use jiff::Timestamp;

use crate::approval::Approver;
use crate::background::Commands;
use crate::bind::Place;
use crate::builtins::Builtins;
use crate::chosen::DefaultModel;
use crate::compaction::Measured;
use crate::defaults::Defaults;
use crate::goal::{GoalProgress, Pursuit};
use crate::homes::{Homes, ShortOfRoom};
use crate::inbox::Inbox;
use crate::memory::MemoryState;
use crate::plan::Planning;
use crate::project_folders::{Brought, ProjectFolders};
use crate::room::LearnedWindows;
use crate::skills::Skills;
use crate::tools::workspace::Workspace;
use crate::toolset::ToolSet;
use crate::waits::Opened;
use crate::worker::Workers;

pub use error::{HarnessError, Invalid};
pub use settings::Settings;

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
    /// The people's computers whose last look at their disks freed nothing, and when.
    pub(crate) lacking_room: Mutex<BTreeMap<ComputerId, Timestamp>>,
    /// Each computer last found short of room with nothing left to free, as this server looked.
    pub(crate) short_of_room: Mutex<BTreeMap<ComputerId, ShortOfRoom>>,
    pub(crate) entropy: Box<dyn Entropy>,
    pub(crate) workspace: Arc<Workspace>,
    /// Where tools come from: the built-in tools first, then any others, such as MCP servers.
    pub(crate) sources: Vec<Arc<dyn ToolSource>>,
    /// The skills on this computer and glave's own (`with_skills`).
    pub(crate) skills: Option<Arc<Skills>>,
    /// The person's own tools, such as their MCP servers', offered after the other sources to
    /// their threads alone: they run with the person's sign-ins. None by default; the host
    /// replaces them when the person's settings change.
    pub(crate) own: RwLock<Vec<Arc<dyn ToolSource>>>,
    /// What threads run with where neither they nor their projects choose: the settings' and
    /// the workspace's at first, then the host's. Each turn takes them as it starts.
    pub(crate) defaults: RwLock<Arc<Defaults>>,
    pub(crate) settings: Settings,
    /// The model threads that chose none go to: the settings' at first, then the host's.
    pub(crate) default: RwLock<DefaultModel>,
    /// The models that read no images, as the host's catalog says or a refusal showed; none
    /// at first.
    pub(crate) without_images: RwLock<Vec<String>>,
    /// Images being made fit to send, [`crate::files::FITTING_AT_ONCE`] at most.
    pub(crate) fitting: tokio::sync::Semaphore,
    /// Who every thread works for, and the agent that runs it: from the settings, or made once.
    pub(crate) actors: Actors,
    /// Each model's real context window, once a refusal has stated it.
    pub(crate) learned_windows: LearnedWindows,
    /// Where threads placed on a worker get one; none by default.
    pub(crate) workers: Option<Workers>,
    /// Each person's own computer, where their threads work; none by default, so every thread
    /// works on the harness's computer.
    pub(crate) homes: Option<Arc<dyn Homes>>,
    /// What threads remember from one to the next; none by default, so the model is sent
    /// exactly what it is sent without memory.
    pub(crate) memory: Option<Arc<dyn Memory>>,
    /// Where threads in a folder of their own read their opening instructions, and what their
    /// project turns on; none by default, so every thread opens with the settings' own and
    /// gets only the harness's tools and hooks.
    pub(crate) project_folders: Option<Arc<dyn ProjectFolders>>,
    /// Workspaces opened for threads let go of while commands were still in them, closed by a
    /// later let-go once their computer has none left there (`bind.rs`).
    pub(crate) lingering: Mutex<Vec<WorkspaceId>>,
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
    /// How many turns it has begun, as its log's `turn.started` count them: a title follows
    /// the first (`title.rs`).
    pub(crate) turns: u64,
    /// The threads that started this one, nearest first: empty unless this is a subagent's
    /// thread, whose prompts an agent, not the person, writes.
    pub(crate) above: Vec<ThreadId>,
    /// The thread's latest goal, active or ended; `None` until one is set.
    pub(crate) goal: Option<Pursuit>,
    /// The thread's plan (`plan.rs`). A subagent's thread keeps none.
    pub(crate) plan: Planning,
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
    /// The mode chosen for the thread (`chosen.rs`); its project's, else the harness's own,
    /// when `None`.
    pub(crate) permissions: Option<Permissions>,
    /// How much its model is asked to reason (`chosen.rs`); the model's own way when `None`.
    pub(crate) effort: Option<String>,
    /// The instructions the thread opens with, and the judge reads as the project's rules:
    /// its folder's, read on its person's own computer when it works there (`homes.rs`), or
    /// the settings' own (`bind.rs`). A subagent's thread has its parent's.
    pub(crate) instructions: Option<String>,
    /// The folder of the project the thread works in, when it works in one here: its own
    /// folder, or the one its worktree was made from. A subagent's thread has its parent's.
    pub(crate) project: Option<PathBuf>,
    /// What its project turned on at the start of the latest turn (`project_folders.rs`). A
    /// subagent's thread has its parent's.
    pub(crate) brought: Arc<Brought>,
    /// The harness's defaults as the latest turn started (`defaults.rs`). A subagent's thread
    /// has its parent's.
    pub(crate) defaults: Arc<Defaults>,
    /// What the person sent its running turn, waiting for the model's next call (`inbox.rs`).
    pub(crate) inbox: Inbox,
    /// Its commands in the background (`background.rs`). A subagent's thread has its own.
    pub(crate) commands: Arc<Commands>,
    /// How its log, as it was read, left its turn: waiting, or left open by a glave that
    /// stopped (`waits.rs`); `None` when no turn was open.
    pub(crate) opened: Option<Opened>,
    /// Where it works now that its project's folder moved, which its next turn records.
    pub(crate) rebound: Option<WorkspaceBound>,
    /// Where it works on its person's own computer, when it does: opened there again at a
    /// turn's start whenever that computer woke since (`homes.rs`).
    pub(crate) home: Option<crate::homes::AtHome>,
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

    /// Whether its log, as it was read, ends in a turn a glave left open, running elsewhere or
    /// stopped without ending it, or may since work on its turn here failed: work on the thread
    /// repairs it first ([`Harness::repair`]).
    pub fn left_open(&self) -> bool {
        matches!(self.opened, Some(Opened::LeftOpen))
    }

    /// Whether its log, as it was read, has its turn waiting for an answer, or answered and
    /// not yet gone on with: it goes on from the log ([`Harness::go_on`]), and no other turn
    /// starts meanwhile.
    pub fn waits(&self) -> bool {
        matches!(self.opened, Some(Opened::Waits | Opened::Answered))
    }

    /// Takes over `before`'s inbox, this thread read again in its place: what was sent to its
    /// turn while it waited reaches the model when the turn goes on.
    pub fn keep_inbox_of(&mut self, before: &Self) {
        self.inbox = before.inbox.clone();
    }

    /// Whose work its model calls are: its organization's, for the person it works for.
    pub(crate) fn payer(&self) -> Payer {
        Payer {
            org: self.scope.org(),
            person: self.for_,
        }
    }

    /// The `seq` its next event takes: one past its last as it was read and written here.
    pub fn next_seq(&self) -> Seq {
        self.next_seq
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
        // Without randomness the actor stays unnamed rather than getting a weak id.
        let made = || {
            let random = parts.entropy.bytes().ok()?;
            PrincipalId::new(parts.clock.now(), random).ok()
        };
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
            lacking_room: Mutex::default(),
            short_of_room: Mutex::default(),
            entropy: parts.entropy,
            skills: None,
            sources: vec![Arc::new(Builtins {
                workspace: Arc::clone(&workspace),
                commands: Arc::default(),
                // Only offered from here: each thread's calls go to its own (`source_for`).
                room: usize::MAX,
            })],
            defaults: RwLock::new(Arc::new(Defaults {
                permissions: settings.permissions,
                highest: settings.highest,
                judge: settings.judge.clone(),
                titles: settings.titles,
                sandbox: workspace.sandbox().cloned(),
                home_sandbox: None,
                ask_network: workspace.asks_network(),
                hooks: Hooks::default(),
                web: None,
            })),
            workspace,
            own: RwLock::default(),
            default: RwLock::new(DefaultModel {
                model: settings.model.clone(),
                provider: settings.provider.clone(),
            }),
            without_images: RwLock::default(),
            fitting: tokio::sync::Semaphore::new(crate::files::FITTING_AT_ONCE),
            settings,
            learned_windows: LearnedWindows::default(),
            workers: None,
            homes: None,
            memory: None,
            project_folders: None,
            lingering: Mutex::default(),
        }
    }

    /// Opens each thread that works in a folder of its own with what `projects` reads for that
    /// folder, rather than the settings' instructions, and gives it what its project turns on
    /// at the start of each turn.
    #[must_use]
    pub fn with_project_folders(
        mut self,
        project_folders: Option<Arc<dyn ProjectFolders>>,
    ) -> Self {
        self.project_folders = project_folders;
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

    /// Works each person's threads on their own computer from `homes`, when there is one.
    #[must_use]
    pub fn with_homes(mut self, homes: Option<Arc<dyn Homes>>) -> Self {
        let sandbox = homes.as_ref().and_then(|homes| homes.sandbox());
        self.change_defaults(|defaults| defaults.home_sandbox = sandbox);
        self.homes = homes;
        self
    }

    /// Offers the tools of `sources` too, after the built-in ones. A tool keeps the first
    /// source that names it.
    #[must_use]
    pub fn with_tools(mut self, sources: Vec<Arc<dyn ToolSource>>) -> Self {
        self.sources.extend(sources);
        self
    }

    /// Offers `web_fetch` from `web`, right after the built-in tools, to every thread whose
    /// project sets up none of its own; none leaves it to the other sources.
    #[must_use]
    pub fn with_web(self, web: Option<Arc<dyn ToolSource>>) -> Self {
        self.change_defaults(|defaults| defaults.web = web);
        self
    }

    /// Offers `skills` to its threads, those on this computer's and glave's own; a thread on a
    /// person's own computer is offered what is found there, with glave's own.
    #[must_use]
    pub fn with_skills(mut self, skills: Option<Skills>) -> Self {
        self.skills = skills.map(Arc::new);
        self
    }

    /// Offers the tools of `sources`, the person's own, to the threads of the person the
    /// harness works for alone, after its other sources.
    #[must_use]
    pub fn with_own_tools(self, sources: Vec<Arc<dyn ToolSource>>) -> Self {
        self.set_own_tools(sources);
        self
    }

    /// Offers the tools of `sources` as the person's own in place of those before, from each
    /// thread's next turn on: a turn already running keeps the tools it started with.
    pub fn set_own_tools(&self, sources: Vec<Arc<dyn ToolSource>>) {
        *self.own.write().unwrap_or_else(PoisonError::into_inner) = sources;
    }

    /// Runs `hooks` at their points of the turns of the person the harness works for.
    #[must_use]
    pub fn with_hooks(self, hooks: Hooks) -> Self {
        self.change_defaults(|defaults| defaults.hooks = hooks);
        self
    }

    /// Every source of `thread`'s tools, in the order a tool keeps the first that names it:
    /// the built-in tools on its workspace, `web_fetch` (its project's, else the harness's),
    /// the harness's other sources, the person's own when the thread is theirs, then its
    /// project's.
    pub(crate) fn sources_for(&self, thread: &Thread) -> Vec<Arc<dyn ToolSource>> {
        let mut sources = self.sources.clone();
        let builtins = crate::builtins::BUILTINS;
        if let (Some(source), Some(own)) =
            (sources.get_mut(builtins), self.source_for(thread, builtins))
        {
            *source = own;
        }
        if let Some(web) = thread.brought.web.as_ref().or(thread.defaults.web.as_ref()) {
            let after = crate::builtins::BUILTINS
                .saturating_add(1)
                .min(sources.len());
            sources.insert(after, Arc::clone(web));
        }
        if let (None, Some(skills)) = (&thread.home, &self.skills) {
            sources.push(Arc::clone(skills) as Arc<dyn ToolSource>);
        }
        if self.trusting(thread.for_) {
            let own = self.own.read().unwrap_or_else(PoisonError::into_inner);
            sources.extend(own.iter().cloned());
        }
        sources.extend(thread.brought.tools.iter().cloned());
        sources
    }

    /// The tools on offer now in `thread`, from every source: the harness's, then its
    /// project's.
    pub(crate) async fn tools(&self, thread: &Thread) -> ToolSet {
        let sources = self.sources_for(thread);
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
        // Plan mode proposes its plan with it, and a thread with a plan keeps it current, so
        // both have it whatever the settings say.
        let planning = self.permissions_for(thread) == Permissions::Plan;
        if self.settings.plans || planning || thread.plan.plan.is_some() {
            set.offer_plan();
        }
        if planning {
            set.system.push_str(crate::plan::PLAN_MODE);
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

    /// The blobs its events refer to.
    pub fn blobs(&self) -> &B {
        &self.blobs
    }

    /// What the harness was set up with: the default model and the mode among them,
    /// as it started. Those now are [`Harness::default_model`] and [`Harness::defaults`].
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
        let id = ThreadId::new(self.clock.now(), self.entropy.bytes()?)?;
        let for_ = caller.person.or(self.actors.person);
        let placed = match place {
            Some(place) => Some(self.place(scope, id, place, for_).await?),
            None => None,
        };
        let (workspace, bound) =
            placed.map_or((None, None), |(workspace, bound)| (workspace, Some(bound)));
        let instructions = self
            .instructions_for(scope, workspace.as_deref(), bound.as_ref(), for_)
            .await;
        // A folder on a person's own computer is not on the server's disk, so the host reads
        // nothing of it.
        let project = bound
            .as_ref()
            .filter(|bound| bound.computer.kind != ComputerKind::Person)
            .and_then(|bound| crate::project_folders::project_of(&bound.opened));
        let mut thread = Thread {
            scope,
            id,
            next_seq: Seq::FIRST,
            messages: Vec::new(),
            allowed: HashSet::new(),
            measured: None,
            request: None,
            asked: Vec::new(),
            turns: 0,
            above: parent.iter().map(|call| call.thread).collect(),
            goal: None,
            plan: Planning::default(),
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
            permissions: None,
            effort: None,
            instructions,
            project,
            brought: Arc::default(),
            defaults: self.defaults(),
            inbox: Inbox::default(),
            commands: Arc::default(),
            opened: None,
            rebound: None,
            home: crate::homes::at_home(bound.as_ref()),
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
        after(thread.next_seq, staged)
    }

    /// Encodes `payload` as the next event of a batch that has not been written yet.
    pub(crate) fn stage<T: Payload>(
        &self,
        thread: &Thread,
        events: &mut Vec<RawEvent>,
        payload: &T,
    ) -> Result<(), HarnessError> {
        self.stage_next(thread.id, thread.next_seq, events, payload)
    }

    /// Like [`Harness::stage`], for thread `thread` whose next event is `next`, as its log has
    /// it rather than as a thread held here does.
    pub(crate) fn stage_next<T: Payload>(
        &self,
        thread: ThreadId,
        next: Seq,
        events: &mut Vec<RawEvent>,
        payload: &T,
    ) -> Result<(), HarnessError> {
        let now = self.clock.now();
        let id = EventId::new(now, self.entropy.bytes()?)?;
        Self::stage_at(thread, next, events, payload, id, now)
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
        let now = self.clock.now();
        Self::stage_at(thread.id, thread.next_seq, events, payload, id, now)
    }

    fn stage_at<T: Payload>(
        thread: ThreadId,
        next: Seq,
        events: &mut Vec<RawEvent>,
        payload: &T,
        id: EventId,
        now: Timestamp,
    ) -> Result<(), HarnessError> {
        let header = Header {
            id,
            thread,
            seq: after(next, events)?,
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

/// The `seq` the next event staged after `staged` gets, in a thread whose next event is `next`.
fn after(next: Seq, staged: &[RawEvent]) -> Result<Seq, HarnessError> {
    let offset = u64::try_from(staged.len()).map_err(|e| HarnessError::Event(e.to_string()))?;
    Seq::new(next.get().saturating_add(offset)).map_err(HarnessError::from)
}
