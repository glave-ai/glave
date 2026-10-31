//! What the server answers: each reply by the shape of its result, and what it carries about
//! the server, threads and turns.

use glave_event::{
    At, EventId, Filters, Money, OrgRole, Origin, Outcome, Principal, RawEvent, Secret, Seq,
    ThreadId, TokenCounts, TurnResult, WorkspaceBound,
};
use glave_harness::{GoalProgress, RunningCommand};
use glave_ports::memories::Remembered;
use glave_ports::{Project, ProjectId, Standing};
use serde::{Deserialize, Serialize};

use super::{
    AcceptedPasskeys, ConnectionInfo, CredentialInfo, CreditsInfo, DeviceCode, FileInfo,
    GoneFolder, Grouping, InviteInfo, ModelInfo, OrganizationInfo, PasskeySummary, Permissions,
    PersonInfo, PlanInfo, ProjectFolder, ProviderInfo, ReplySources, ScheduleInfo, SearchInfo,
    ShownInvite, SignInLink, SignInWay, TaskInfo, ThreadChanges, ThreadSteps, Told, TrustAsk,
    WatchEnd,
};

/// What the server is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    /// The protocol versions spoken.
    pub protocols: Vec<String>,
    /// What the agent may do in a thread that, and whose project, chose no mode:
    /// `--permissions` or `permissions` in `config.toml` (`specs/approvals.md`, "Modes").
    pub permissions: Permissions,
    /// The modes a thread may choose (`permissions` on `thread/start` and `turn/start`):
    /// those the policy leaves, in the order a person is offered them, the recommended first.
    pub modes: Vec<Permissions>,
    /// Why Auto asks before every change and command here, in a sentence, when it does:
    /// no sandbox can run on this computer (Ask first then asks so too), or only one that keeps
    /// files as they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsandboxed: Option<Told>,
    /// The server's id, where it signs browsers in: the same across restarts, and the one a
    /// browser's session must name before the browser presents it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Where people reach the server from elsewhere, if they do: its public origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Where a browser on this computer makes and signs in with passkeys, when the server
    /// makes them here rather than at a public origin: `http://localhost:<port>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub here: Option<String>,
    /// How a browser signs in to it; none where no browser does, as over stdio. A passkey: at a
    /// public origin always, a setup link making the first; on this computer once one is made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signin: Vec<SignInWay>,
    /// What it has beyond the one person a server works for on its own; absent where it has
    /// none of it, as on a person's own computer or server.
    #[serde(default, skip_serializing_if = "Exists::is_none")]
    pub exists: Exists,
    /// The caller's role in the organization, when there is one and they signed in: so a
    /// client shows admin screens only to its Admins and Owners. Absent on a server with no
    /// organization, where the one person may do everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<OrgRole>,
    /// Whether it sends email (`[email]`) with links to its public origin, as an invite's: so
    /// a client asks for an address only where one is used.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub email: bool,
    /// Whether the caller may invite someone here: the server takes invites (glave accounts
    /// at a public origin, or a company login), and the caller's role permits it. So a person
    /// alone, whose organization is shown to no one, can still make the first.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invites: bool,
}

/// What a server has beyond a person's own: each because its configuration provides it, never
/// a setting naming a kind of deployment (`specs/deployments.md`, "One code path"). A client
/// draws only what is here, and nothing of what is not.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Exists {
    /// An organization of people, who sign in through its identity provider (OIDC).
    pub organization: bool,
    /// Workers: a fresh machine for each thread of the project whose settings chose them.
    pub workers: bool,
    /// Credits: each organization pays ahead for what it uses, and its model calls are
    /// charged to them (`credits/get`). Absent where they are not, so what a server that
    /// keeps none says is as it was.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub credits: bool,
}

impl Exists {
    /// Whether it has none of it.
    pub(crate) fn is_none(&self) -> bool {
        *self == Self::default()
    }
}

/// Whether this server keeps its computer from sleeping on its own while a turn, task or
/// schedule runs (`specs/one-server.md`, owner decision 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AwakeInfo {
    /// Never: a server, which does not sleep. There is nothing to choose.
    Never,
    /// On a person's own computer, as they choose: kept awake while glave works when `on`, the
    /// screen still free to sleep and closing the lid still sleeping it.
    Chosen { on: bool },
    /// On a person's own computer that glave has nothing to keep awake with: why, in a
    /// sentence.
    Cannot { reason: Told },
}

/// Whether glave remembers for the person (`specs/memory.md`, "Defaults and policy").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RememberingInfo {
    /// Not the caller's to choose here, and `reason`, why: their organization turned memory
    /// off, or only the person on this computer chooses.
    Never { reason: Told },
    /// The person's to turn on or off: on when `on`.
    Chosen { on: bool },
}

/// A thread, as a list shows it: from the threads view, with what it is doing now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ThreadSummary {
    pub id: ThreadId,
    pub project: ProjectId,
    pub status: Status,
    pub standing: Standing,
    /// The title the person gave it, else the first line of their first message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What the agent may do in its next turn: the mode last chosen for it, else its
    /// project's, else the server's.
    pub permissions: Permissions,
    /// What its project asks the person on this computer to trust, once the thread was opened
    /// or started: shown to them alone, until they answer it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<TrustAsk>,
    /// Whose work it is; absent in threads from before it was recorded (the local person).
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub for_: Option<Principal>,
    pub origin: Origin,
    /// The thread whose call started this one, for a subagent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
    pub created_at: At,
    pub updated_at: At,
    pub turns: u64,
    /// How its latest turn ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// Tokens its own model calls used.
    pub tokens: TokenCounts,
    /// What they cost, when every call reported a cost; absent when one did not: the cost is
    /// not reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    /// True when a usage count in `tokens`, and so `cost`, was estimated: a stream ended before
    /// its provider reported it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
    /// Where it works, once it has a workspace, as its latest `workspace.bound` records it: the
    /// computer (the one the server runs on, or a worker), the folder its tools act in, and how
    /// it got it (its project's folder in place, a worktree on a branch of its own, a clone, or
    /// a scratch folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Box<WorkspaceBound>>,
    /// How full its context is, as compaction measures it; absent before its first model call
    /// and since it was compacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextUse>,
}

/// How full a thread's context is: what its conversation last sent and got back, of the window
/// compaction aims for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ContextUse {
    /// The tokens its latest call sent and got back.
    pub tokens: u64,
    /// The window compaction aims for, in tokens: the configured one, or a smaller one the
    /// model stated.
    pub window: u64,
    /// `tokens` of `window`, in whole percent, at most 100. A turn compacts before a request
    /// past its threshold (80 unless set).
    pub percent: u8,
}

impl ContextUse {
    /// `tokens` of a `window` of tokens.
    pub(crate) fn of(tokens: u64, window: u64) -> Self {
        let share = tokens.saturating_mul(100) / window.max(1);
        Self {
            tokens,
            window,
            percent: u8::try_from(share.min(100)).unwrap_or(100),
        }
    }
}

/// What a thread is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Running,
    /// A turn is paused until someone answers an approval or a question, or the thread's
    /// project asks the person to trust it.
    Waiting,
}

/// A message queued to the turn running in its thread, for the model's next call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct QueuedMessage {
    /// The id of the `message.added` it is recorded as once the model reads it: that
    /// `thread/event` says it was read. A `turn/live` `unread` with this id says it never will be.
    pub id: EventId,
}

/// A reply, by the shape of its result.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum Reply {
    #[schemars(title = "Server")]
    Server(ServerInfo),
    #[schemars(title = "Thread")]
    Thread { thread: ThreadSummary },
    #[schemars(title = "Threads")]
    Threads {
        threads: Vec<ThreadSummary>,
        /// The cursor of the next page, when there is one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
    },
    #[schemars(title = "Events")]
    Events {
        events: Vec<RawEvent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<Seq>,
    },
    #[schemars(title = "Turn")]
    Turn { turn: TurnResult },
    /// A file kept for a message.
    #[schemars(title = "File")]
    File { file: FileInfo },
    /// A message sent while a turn ran, queued to it: answered at once.
    #[schemars(title = "Queued")]
    Queued { queued: QueuedMessage },
    #[schemars(title = "Compacted")]
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
    },
    #[schemars(title = "Task")]
    Task { task: TaskInfo },
    #[schemars(title = "Tasks")]
    Tasks { tasks: Vec<TaskInfo> },
    #[schemars(title = "Schedule")]
    Schedule { schedule: ScheduleInfo },
    #[schemars(title = "Schedules")]
    Schedules { schedules: Vec<ScheduleInfo> },
    /// A project: one asked for, made or renamed, with its folder when it has one.
    #[schemars(title = "Project")]
    Project {
        project: Project,
        /// Where its threads work.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<ProjectFolder>,
    },
    #[schemars(title = "Projects")]
    Projects {
        projects: Vec<Project>,
        /// The projects among them whose folder isn't there anymore.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        gone: Vec<GoneFolder>,
    },
    /// The folders inside the folder at `path`, by name; hidden ones left out.
    #[schemars(title = "Folders")]
    Folders {
        /// The folder listed, as this computer names it, links followed.
        path: String,
        /// The folder it is in, to go up to; absent at the top.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        folders: Vec<String>,
        /// Why the folder listed is no project's folder, when it is none: the server's
        /// sentence. The home folder, a folder above it and the temporary folder never are
        /// (`project/create` refuses them), though the folders inside may be.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refused: Option<Told>,
    },
    /// Memories listed, or the ones a change touched. A memory's `source` names the chat it
    /// was saved in only when that chat was started for the caller and is still there.
    #[schemars(title = "Memories")]
    Memories { memories: Vec<Remembered> },
    /// The models a thread may be sent to, the default first.
    #[schemars(title = "Models")]
    Models { models: Vec<ModelInfo> },
    /// A model account, as it was connected.
    #[schemars(title = "Provider")]
    Provider { provider: ProviderInfo },
    #[schemars(title = "Providers")]
    Providers { providers: Vec<ProviderInfo> },
    /// A git host, as it was connected.
    #[schemars(title = "Credential")]
    Credential { credential: CredentialInfo },
    #[schemars(title = "Credentials")]
    Credentials {
        credentials: Vec<CredentialInfo>,
        /// Where the person chooses which repositories GitHub's app may use, when it has
        /// such a page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        install: Option<String>,
        /// Where signing in to GitHub in the browser goes, as an origin: the host whose
        /// credential is the one GitHub's sign-in made.
        github: String,
    },
    /// A connection, as it was signed in to.
    #[schemars(title = "Connection")]
    Connection { connection: ConnectionInfo },
    #[schemars(title = "Connections")]
    Connections { connections: Vec<ConnectionInfo> },
    /// Where the person signs in to a remote server.
    #[schemars(title = "SignIn")]
    SignIn { sign_in: SignInLink },
    #[schemars(title = "Commands")]
    Commands { commands: Vec<RunningCommand> },
    #[schemars(title = "Device")]
    Device { device: DeviceCode },
    /// A sign-in in the browser still waiting for the person.
    #[schemars(title = "Waiting")]
    Waiting { flow: String },
    /// A link's one-time code, sent once.
    #[schemars(title = "Link")]
    Link {
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        code: Secret,
    },
    /// A request to make or use a passkey: its id, and the options the browser hands its
    /// authenticator, as WebAuthn's JSON (`{"publicKey": …}`, buffers in base64url).
    #[schemars(title = "PasskeyOptions")]
    PasskeyOptions {
        ceremony: String,
        options: serde_json::Value,
    },
    #[schemars(title = "Passkey")]
    Passkey { passkey: PasskeySummary },
    /// The person's passkeys; for a glave account, the address it gets back in with, and what
    /// it is told of a passkey made by email lately on another device.
    #[schemars(title = "Passkeys")]
    Passkeys {
        passkeys: Vec<PasskeySummary>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        email: Option<String>,
        /// The name a glave account's person goes by, when they gave one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        person_name: Option<String>,
        /// Whether they may change it (`person/name`): they have a glave account.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        nameable: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notice: Option<Told>,
    },
    /// A browser session, sent once.
    #[schemars(title = "Browser")]
    Browser {
        /// The bearer token the browser signs in with, kept in its origin's storage for every
        /// tab, never in a cookie.
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        session: Secret,
        /// The id of the server it is for: the browser presents it to no other.
        server: String,
        /// Every passkey that signs the person in, for the browser's passkey manager; none
        /// where the server makes no passkeys for the page signed in, as after a link on a
        /// server whose passkeys are made at its public origin.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        accepted: Option<AcceptedPasskeys>,
    },
    /// How a watch ended: its stream's last message.
    #[schemars(title = "Watched")]
    Watched { ended: WatchEnd },
    // Its one field is always sent, and needed to read it, so no other reply reads as this one.
    /// The language the caller chose, as a BCP 47 tag; `null` while their client's counts.
    #[schemars(title = "Language")]
    Language {
        #[serde(deserialize_with = "Option::deserialize")]
        language: Option<String>,
    },
    /// How the caller's clients group their threads.
    #[schemars(title = "Grouping")]
    Grouping { grouping: Grouping },
    /// The caller's filters among a router's providers, and the countries, by ISO 3166-1
    /// alpha-2 code, that "Europe" means among them: the European Economic Area, Switzerland
    /// and the United Kingdom.
    #[schemars(title = "Filters")]
    Filters {
        filters: Filters,
        europe: Vec<String>,
    },
    /// Whether the computer is kept awake while glave works.
    #[schemars(title = "Awake")]
    Awake { awake: AwakeInfo },
    /// Whether glave remembers for the person.
    #[schemars(title = "Remembering")]
    Remembering { remembering: RememberingInfo },
    /// Whether the person's agent may search the web.
    #[schemars(title = "Search")]
    Search { search: SearchInfo },
    /// What a thread's replies drew on from the web.
    #[schemars(title = "Sources")]
    Sources { sources: Vec<ReplySources> },
    /// What a thread changed in files.
    #[schemars(title = "Changes")]
    Changes { changes: ThreadChanges },
    /// A thread's turns in steps.
    #[schemars(title = "Steps")]
    Steps { steps: ThreadSteps },
    /// The organization's people, in id order.
    #[schemars(title = "People")]
    People { people: Vec<PersonInfo> },
    /// A person, as they are now.
    #[schemars(title = "Person")]
    Person { person: PersonInfo },
    /// An invite just made, and its code, sent this once.
    #[schemars(title = "Invited")]
    Invited {
        invite: InviteInfo,
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        code: Secret,
    },
    /// The invites waiting to be opened, oldest first.
    #[schemars(title = "Invites")]
    Invites { invites: Vec<InviteInfo> },
    /// What an invite's link opens.
    #[schemars(title = "InviteShown")]
    InviteShown { shown: ShownInvite },
    /// The organizations the caller belongs to, their own first.
    #[schemars(title = "Organizations")]
    Organizations {
        organizations: Vec<OrganizationInfo>,
    },
    /// The organization's credits, as much as the caller may see.
    #[schemars(title = "Credits")]
    Credits { credits: CreditsInfo },
    // After every reply with a field of its own, since a missing `goal` or `plan` reads as
    // `null`; before `Done`, which takes any object.
    /// The thread's latest goal, or `null`.
    #[schemars(title = "Goal")]
    Goal { goal: Option<GoalProgress> },
    /// The thread's plan, or `null`.
    #[schemars(title = "Plan")]
    Plan { plan: Option<PlanInfo> },
    #[schemars(title = "Done")]
    Done {},
}
