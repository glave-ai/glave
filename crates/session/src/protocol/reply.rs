//! What the server answers: each reply by the shape of its result, and what it carries about
//! the server, threads and turns.

use glave_event::{
    At, EventId, Money, Origin, Outcome, Principal, RawEvent, Secret, Seq, ThreadId, TokenCounts,
};
use glave_harness::{GoalProgress, RunningCommand, TurnSummary};
use glave_ports::memories::Remembered;
use glave_ports::{Project, ProjectId, Standing};
use serde::{Deserialize, Serialize};

use super::{
    ConnectionInfo, CredentialInfo, DeviceCode, FileInfo, GoneFolder, Grouping, ModelInfo,
    PasskeySummary, Permissions, ProjectFolder, ProviderInfo, ScheduleInfo, SignInLink, SignInWay,
    TaskInfo, Told, TrustAsk, WatchEnd,
};

/// What the server is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    /// The protocol versions spoken.
    pub protocols: Vec<String>,
    /// What the agent may do in a thread that, and whose project, chose no preset:
    /// `--permissions` or `permissions` in `config.toml` (`specs/approvals.md`, "Permissions").
    pub permissions: Permissions,
    /// The presets a thread may choose (`permissions` on `thread/start` and `turn/start`):
    /// those the policy leaves, the most cautious first.
    pub presets: Vec<Permissions>,
    /// Why Workspace asks before every change and command here, in a sentence, when it does:
    /// no sandbox can run on this computer (Read-only then asks so too), or only one that keeps
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
    Cannot { reason: String },
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
    /// What the agent may do in its next turn: the preset last chosen for it, else its
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
    /// What they cost, when every call reported a cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
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

/// How a turn ended and what it cost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TurnResult {
    pub outcome: Outcome,
    pub model_calls: u32,
    pub tool_calls: u32,
    /// The commands that ran outside the sandbox, each one allowed.
    #[serde(default)]
    pub outside_sandbox: u32,
    /// The commands that ran with the network the sandbox otherwise kept from them, each one
    /// allowed.
    #[serde(default)]
    pub with_network: u32,
    pub tokens: TokenCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    /// True when a usage count was estimated, not reported.
    #[serde(default)]
    pub estimated: bool,
}

impl From<TurnSummary> for TurnResult {
    fn from(summary: TurnSummary) -> Self {
        Self {
            outcome: summary.outcome,
            model_calls: summary.model_calls,
            tool_calls: summary.tool_calls,
            outside_sandbox: summary.outside_sandbox,
            with_network: summary.with_network,
            tokens: summary.tokens,
            cost: summary.cost,
            estimated: summary.estimated,
        }
    }
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
        /// Whether GitHub's sign-in in the browser (`credential/device/*`) is set up here;
        /// without it, a GitHub token is pasted.
        device: bool,
        /// Where the person chooses which repositories GitHub's app may use, when it has
        /// such a page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        install: Option<String>,
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
    #[schemars(title = "Passkeys")]
    Passkeys { passkeys: Vec<PasskeySummary> },
    /// A browser session, sent once.
    #[schemars(title = "Browser")]
    Browser {
        /// The bearer token the browser signs in with, kept in its origin's storage for every
        /// tab, never in a cookie.
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        session: Secret,
        /// The id of the server it is for: the browser presents it to no other.
        server: String,
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
    /// Whether the computer is kept awake while glave works.
    #[schemars(title = "Awake")]
    Awake { awake: AwakeInfo },
    // After every reply with a field of its own, since a missing `goal` reads as `null`;
    // before `Done`, which takes any object.
    /// The thread's latest goal, or `null`.
    #[schemars(title = "Goal")]
    Goal { goal: Option<GoalProgress> },
    #[schemars(title = "Done")]
    Done {},
}
