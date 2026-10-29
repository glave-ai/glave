//! The messages of the session API (`specs/session.md`): requests by method, their replies, the
//! notifications a turn sends, and errors. Every transport carries these same types; in-process
//! they are passed as they are, elsewhere as JSON-RPC 2.0.

mod acts_on;
mod changes;
pub mod code;
mod connections;
mod credentials;
mod error;
mod files;
mod live;
mod models;
mod passkeys;
mod people;
mod plans;
mod projects;
mod providers;
mod reply;
mod settings;
mod sources;
mod steps;
mod tasks;
mod told;
mod trust;

use glave_event::{
    At, CallId, CredentialId, InviteId, MemoryId, MemoryScope, OrgRole, PrincipalId, ProviderId,
    RawEvent, ScheduleId, Secret, Seq, ThreadId, When,
};
use glave_ports::ProjectId;
use glave_ports::computer::{CommandId, ComputerId};
use serde::{Deserialize, Serialize};

pub use changes::{
    ChangeKind, ChangedFile, DiffLine, FileChange, LineKind, ThreadChanges, TurnChanges,
};
pub use connections::{ConnectionInfo, ConnectionStatus, SearchInfo, SignInLink, Usage};
pub use credentials::{Ability, CredentialInfo, DeviceCode};
pub use error::{ApiError, ErrorData};
pub use files::{AttachedFile, FileInfo};
pub use glave_event::{Forge, ProviderKind};
pub use glave_event::{Grouping, Permissions, TurnResult};
pub use live::Update;
pub use models::ModelInfo;
pub use passkeys::{AcceptedPasskeys, LinkKind, PasskeySummary, SignInWay, UnknownPasskey};
pub use people::{InviteInfo, JoinWay, PersonInfo, ShownInvite};
pub use plans::PlanInfo;
pub use projects::{GoneFolder, NewFolder, ProjectFolder};
pub use providers::{ProviderInfo, hint};
pub use reply::{
    AwakeInfo, ContextUse, Exists, QueuedMessage, RememberingInfo, Reply, ServerInfo, Status,
    ThreadSummary,
};
pub use settings::Setting;
pub use sources::ReplySources;
pub use steps::{CallRun, ThreadSteps};
pub use tasks::{ScheduleInfo, TaskInfo, TaskKind, TaskStatus};
pub use told::Told;
pub use trust::{TrustAsk, TrustChoice, TrustItem};

/// The protocol version this server speaks.
pub const PROTOCOL: &str = "2026-10";

/// The one table of methods: each method's name, its request's params, and the replies it
/// answers with, by the titles of their [`Reply`] variants. It makes [`Request`], whose serde
/// names are the methods, [`Request::method`], and [`RESULTS`] for the schema.
macro_rules! methods {
    ($(
        $(#[$doc:meta])*
        $method:literal => $variant:ident { $($params:tt)* } -> [$($reply:ident),+],
    )*) => {
        /// A request, by method.
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(tag = "method", content = "params")]
        pub enum Request {
            $(
                $(#[$doc])*
                #[serde(rename = $method)]
                $variant { $($params)* },
            )*
        }

        impl Request {
            /// The method's name, as JSON-RPC and the `Glave-Method` header carry it.
            pub fn method(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $method,)*
                }
            }
        }

        /// Every method, with the titles of the replies it answers with.
        pub(crate) const RESULTS: &[(&str, &[&str])] = &[
            $(($method, &[$(stringify!($reply)),+]),)*
        ];
    };
}

methods! {
    /// What the server is. Over HTTP it needs no sign-in, so a browser learns which server it
    /// reached before it presents a session.
    "server/discover" => Discover {} -> [Server],
    /// Whether this server keeps its computer awake while it works: on a person's own computer,
    /// as they choose.
    "awake/get" => AwakeGet {} -> [Awake],
    /// Chooses whether the person's own computer is kept awake while glave works, from the
    /// next work on. Only the person on this computer, and only where it is theirs to choose.
    "awake/set" => AwakeSet { on: bool } -> [Awake],
    /// The language the caller chose to read glave's own sentences in, if they chose one.
    "language/get" => LanguageGet {} -> [Language],
    /// Chooses the language glave's own sentences are written in for the caller, as a BCP 47
    /// tag (`en`, `en-GB`); with none, their client's (`Accept-Language`) again. Refused for a
    /// language glave does not write in.
    "language/set" => LanguageSet {
        #[serde(default)]
        language: Option<String>,
    } -> [Language],
    /// How the caller's clients group their threads: by project until they choose.
    "grouping/get" => GroupingGet {} -> [Grouping],
    /// Chooses how the caller's clients group their threads: by project or by status. The
    /// server keeps the choice; a client groups by what each thread is doing, as it says.
    "grouping/set" => GroupingSet { grouping: Grouping } -> [Grouping],
    /// A one-time code for a link that signs a browser on this computer in: good once, for
    /// five minutes. Only glave on this computer: a local transport, or the local token.
    "signin/link" => SigninLink {} -> [Link],
    /// Trades a link's code for a browser session with this server, from this computer: a
    /// bearer token, good for 30 days from its last use. Over HTTP it needs no sign-in.
    "signin/redeem" => SigninRedeem {
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        code: Secret,
    } -> [Browser],
    /// Ends this browser's session: signing out.
    "signin/end" => SigninEnd {} -> [Done],
    /// Ends every browser session but this one's: signing other browsers out.
    "signin/end_others" => SigninEndOthers {} -> [Done],
    /// A one-time code for a link that makes a passkey on a device elsewhere, at the server's
    /// public origin: good once, for ten minutes. Only glave on this computer: a local
    /// transport, or the local token.
    "signin/setup" => SigninSetup {} -> [Link],
    /// Begins making a passkey with a setup link's code, or an invite's or a sign-up's: the
    /// first passkey of a new glave account. Over HTTP it needs no sign-in.
    "passkey/create_options" => PasskeyCreateOptions {
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        code: Secret,
        /// Which link `code` is: a setup link's unless said.
        #[serde(default, skip_serializing_if = "LinkKind::is_setup")]
        link: LinkKind,
    } -> [PasskeyOptions],
    /// Makes the passkey that answers a setup link's request, spends the code, and signs the
    /// browser in. Over HTTP it needs no sign-in.
    "passkey/create" => PasskeyCreate {
        /// The request, as `passkey/create_options` named it.
        ceremony: String,
        /// The browser's answer, as WebAuthn's JSON: `PublicKeyCredential`'s, buffers in
        /// base64url.
        credential: serde_json::Value,
        /// What to call the passkey; when it was added, if nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    } -> [Browser],
    /// Begins adding a passkey on this device, for the person signed in.
    "passkey/add_options" => PasskeyAddOptions {} -> [PasskeyOptions],
    /// Adds the passkey that answers the person's request.
    "passkey/add" => PasskeyAdd {
        ceremony: String,
        credential: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    } -> [Passkey],
    /// Begins signing a browser in with a passkey. Over HTTP it needs no sign-in.
    "passkey/get_options" => PasskeyGetOptions {} -> [PasskeyOptions],
    /// Signs a browser in with the passkey that answers the request. Over HTTP it needs no
    /// sign-in.
    "passkey/get" => PasskeyGet {
        ceremony: String,
        credential: serde_json::Value,
    } -> [Browser],
    /// The person's passkeys, oldest first.
    "passkey/list" => PasskeyList {} -> [Passkeys],
    /// Removes a passkey, and ends the browser sessions it began.
    "passkey/remove" => PasskeyRemove { passkey: String } -> [Done],
    /// The organization's people who signed in through its identity provider, with their
    /// roles, in id order after `after` (someone listed before), at most `limit` (100 unless
    /// said, at most 1,000). Only for its Admins and Owners.
    "people/list" => PeopleList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<PrincipalId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    } -> [People],
    /// Gives a person who signed in a role. Only for the organization's Admins and Owners, and
    /// only an Owner makes an Owner or changes one's role.
    "role/set" => RoleSet { person: PrincipalId, role: OrgRole } -> [Person],
    /// Takes someone out of the organization: their sessions and watches in it end at once, and
    /// they come back only by an invite. Only for the organization's Admins and Owners; only an
    /// Owner removes an Owner, and never the last.
    "person/remove" => PersonRemove { person: PrincipalId } -> [Done],
    /// Makes an invite to the organization giving `role`, good once and for 7 days: answered
    /// with its code, this once, for a link (`<origin>/#invite=<code>`). Only for those whose
    /// role permits inviting, and only an Owner invites an Owner.
    "invite/create" => InviteCreate {
        role: OrgRole,
        /// Whom to email its link to, where the server sends email; else it is the caller's to
        /// share. The address is not kept.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        email: Option<String>,
    } -> [Invited],
    /// The invites waiting to be opened, oldest first, never their codes.
    "invite/list" => InviteList {} -> [Invites],
    /// Revokes an invite waiting to be opened.
    "invite/revoke" => InviteRevoke { invite: InviteId } -> [Done],
    /// Signs up with `email`, where the server takes sign-ups: a link sent there makes the
    /// person's account and an organization of their own. Answered the same whether or not an
    /// account has that email. Over HTTP it needs no sign-in.
    "signup/start" => SignupStart { email: String } -> [Done],
    /// What an invite's link opens, before anyone signs in: the organization it is to, the role
    /// it gives, who made it, and how someone joins with it. Over HTTP it needs no sign-in.
    "invite/show" => InviteShow {
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        code: Secret,
    } -> [InviteShown],
    /// Opens an invite with its link's `code`, as the person signed in through the
    /// organization's identity provider, or with a glave account, who joins the organization
    /// it is to: their role is raised to the invite's, and the invite is spent.
    "invite/accept" => InviteAccept {
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        code: Secret,
    } -> [Person],
    /// The models this server can run: the default first.
    "model/list" => ModelList {} -> [Models],
    "thread/start" => ThreadStart {
        /// The project the thread is in: by default the caller's personal project, their Chats.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
        /// Whether the thread works in a worktree of its own rather than in the folder itself;
        /// the server's default (`worktree` in `config.toml`) when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<bool>,
        /// For a thread that runs as a task: in a project with a folder it works on a copy
        /// of its own, a worktree (or a worker, where every thread works on one), never in
        /// the folder itself, so `worktree: false` is refused.
        #[serde(default, skip_serializing_if = "is_false")]
        background: bool,
        /// A running computer to work on, such as the person's own laptop dialled in to this
        /// server. Not available yet: refused with `-32022`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        computer: Option<ComputerId>,
        /// The model the thread's turns go to, one `model/list` names; the server's default
        /// when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// What the agent may do in the thread's turns, one of the server's `modes`; its
        /// project's, else the server's own, when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permissions: Option<Permissions>,
        /// How much the model reasons in the thread's turns: one of the levels `model/list`
        /// gives for its model, such as `high`. The model's own default when absent or `null`.
        #[serde(default, deserialize_with = "given", skip_serializing_if = "Option::is_none")]
        effort: Option<Option<String>>,
    } -> [Thread],
    "thread/list" => ThreadList {
        /// The project whose threads to list, by default as `thread/start` picks one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
        /// Which threads: the active ones by default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        show: Option<Show>,
        /// Where the page starts: the `next` of the page before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    } -> [Threads],
    /// Gives a thread a title of the person's own.
    "thread/rename" => ThreadRename { thread: ThreadId, title: String } -> [Thread],
    /// Hides a thread from lists, keeping it in the log.
    "thread/archive" => ThreadArchive { thread: ThreadId } -> [Done],
    /// Brings an archived thread back.
    "thread/unarchive" => ThreadUnarchive { thread: ThreadId } -> [Done],
    /// Hides a thread for good and marks it for erasure.
    "thread/delete" => ThreadDelete { thread: ThreadId } -> [Done],
    "thread/get" => ThreadGet { thread: ThreadId } -> [Thread],
    "thread/events" => ThreadEvents {
        thread: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<Seq>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    } -> [Events],
    /// What a thread's replies drew on from the web, read from its log: for each reply whose
    /// turn read pages with `web_fetch` before it, those pages, in order.
    "thread/sources" => ThreadSources { thread: ThreadId } -> [Sources],
    /// What a thread changed in files with glave's own `write` and `edit`, its subagents'
    /// changes with its own, read from the logs: each file with the lines added and taken out
    /// and each change, for the thread and for each turn that changed any.
    "thread/changes" => ThreadChanges { thread: ThreadId } -> [Changes],
    /// A thread's turns in steps, read from its log: each run of tool calls with nothing said
    /// between them, its calls and what they did in a line, and which message is each turn's
    /// reply.
    "thread/steps" => ThreadSteps { thread: ThreadId } -> [Steps],
    /// The thread's plan, read from its log: its steps, each with its status, and whether it
    /// waits for the person's approval; `null` when it has none.
    "thread/plan" => ThreadPlan { thread: ThreadId } -> [Plan],
    /// Follows a thread: its events after `after` from the log, then its events as they are
    /// recorded and its live updates, its subagents' with them, each event once and in order.
    /// Streams as `turn/start` does, until the client ends the watch (`thread/unwatch`, or
    /// closing the stream), or the thread is deleted. A connection of lines watches each thread
    /// once, at most 64 at a time.
    "thread/watch" => ThreadWatch {
        thread: ThreadId,
        /// The last `seq` the client has: only the events after it are sent. Every event when
        /// absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<Seq>,
    } -> [Watched],
    /// Ends this connection's watch of a thread, which then answers its request. Over HTTP,
    /// where each watch is a request of its own, closing its stream ends it.
    "thread/unwatch" => ThreadUnwatch { thread: ThreadId } -> [Done],
    /// Follows the caller's threads as `thread/list` shows them: each one's summary
    /// (`thread/changed`) whenever it changes, whatever its standing, until the client ends the
    /// watch. Streams as `thread/watch` does; a connection of lines watches each list once,
    /// within its 64 watches.
    "thread/list/watch" => ThreadListWatch {
        /// The project whose threads to follow: every project the caller owns when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
        /// The latest `updated_at` the client has: the threads updated since are sent first,
        /// read from the threads view, oldest first. Only what changes from now when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<At>,
    } -> [Watched],
    /// Ends this connection's watch of a list, which then answers its request. Over HTTP,
    /// closing its stream ends it.
    "thread/list/unwatch" => ThreadListUnwatch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
    } -> [Done],
    /// Sends the person's message: it starts a turn, or while one runs it is queued to it, and
    /// the model reads it at its next call.
    "turn/start" => TurnStart {
        thread: ThreadId,
        prompt: String,
        /// Files the person added to the message, each one `file/upload` kept in the thread's
        /// project: the thread refers to them from now on, and the model is sent them as it can
        /// read them. Not with `background`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<AttachedFile>,
        /// Runs the turn as a task, with no one attached: answered at once with the task.
        #[serde(default, skip_serializing_if = "is_false")]
        background: bool,
        /// With `background`, tokens the turn may use, subagents' included.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<u64>,
        /// The model this turn, and the thread's turns after it, go to: one `model/list`
        /// names. The thread's latest choice when absent, else the server's default. Not while
        /// a turn runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        /// How much the model reasons in this turn, and the thread's turns after it: one of
        /// the levels `model/list` gives for the model the turn goes to, or `null` for Default,
        /// the model's own default again, as in a thread that never chose. The thread's latest
        /// choice when absent, else the model's own default. Not while a turn runs.
        #[serde(default, deserialize_with = "given", skip_serializing_if = "Option::is_none")]
        effort: Option<Option<String>>,
        /// What the agent may do in this turn, and the thread's turns after it: one of the
        /// server's `modes`. The thread's latest choice when absent, else its project's, else
        /// the server's. Not while a turn runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permissions: Option<Permissions>,
        /// No one answers the turn it starts: what would ask is declined at once and
        /// `question` is not offered, as in a task, for its subagents too, while it still
        /// streams to whoever follows it. For a client with no one at it, such as a script.
        #[serde(default, skip_serializing_if = "is_false")]
        unattended: bool,
        #[serde(rename = "_meta", default, skip_serializing_if = "Meta::is_empty")]
        meta: Meta,
    } -> [Turn, Task, Queued],
    "turn/cancel" => TurnCancel { thread: ThreadId } -> [Done],
    /// Keeps a file the person added to a message they are writing, in `project` (their Chats
    /// when absent), for `turn/start` to name: kept a day unless a message is sent with it.
    /// Images (PNG, JPEG, GIF, WebP), PDFs and text, each up to 10 MB, kept as what they hold
    /// says they are; anything else is refused, videos with a sentence of their own.
    "file/upload" => FileUpload {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
        /// Its name, as the person's computer called it.
        name: String,
        /// Its media type, such as `image/png`; `application/octet-stream` when unknown. What
        /// it holds decides whether it is an image or a PDF; this names a text file's kind.
        media_type: String,
        /// Its content, in base64.
        data: String,
    } -> [File],
    "thread/compact" => ThreadCompact { thread: ThreadId } -> [Compacted],
    "approval/decide" => ApprovalDecide {
        thread: ThreadId,
        approval: CallId,
        decision: Answer,
    } -> [Done],
    /// Answers the model's questions: one answer for each, in order.
    "question/answer" => QuestionAnswer {
        thread: ThreadId,
        question: CallId,
        answers: Vec<String>,
    } -> [Done],
    /// Answers what a thread's project asks to be trusted (its summary's `trust`): `trust`
    /// turns on what it listed from the thread's next turn and keeps the answer; `not_now`
    /// keeps it off and stops asking in this thread. Only the person on this computer.
    "trust/answer" => TrustAnswer {
        thread: ThreadId,
        /// The ask answered: its `ask`.
        ask: String,
        answer: TrustChoice,
    } -> [Thread],
    /// Sets a goal and works toward it across turns; without `objective`, continues the
    /// thread's active goal, as after a restart, or resumes the one the model paused.
    "goal/start" => GoalStart {
        thread: ThreadId,
        /// What to achieve, with how to verify it in the person's words.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        objective: Option<String>,
        /// Tokens the goal may use; glave's default when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<u64>,
        /// Works toward the goal as a task, with no one attached: answered at once.
        #[serde(default, skip_serializing_if = "is_false")]
        background: bool,
        /// No one answers its turns, as `turn/start`'s `unattended` says.
        #[serde(default, skip_serializing_if = "is_false")]
        unattended: bool,
    } -> [Turn, Task],
    /// Stops the thread's active goal, cancelling its running turn.
    "goal/stop" => GoalStop { thread: ThreadId } -> [Done],
    /// Replaces the steps of the thread's proposed plan with the person's, each to do, and
    /// answers with the plan: the model is told at its next turn, and builds from them. Only
    /// while a plan waits for approval, and not while a turn runs.
    "plan/edit" => PlanEdit {
        thread: ThreadId,
        /// The steps, in order: 1 to 64, each a line of up to 1,000 characters.
        steps: Vec<String>,
    } -> [Plan],
    /// Builds the thread's proposed plan: records the approval, runs the thread in Auto from
    /// now on (or the highest mode the policy leaves, if lower), and starts the turn that
    /// builds it from the plan as it is, streamed as `turn/start` streams one.
    "plan/build" => PlanBuild { thread: ThreadId } -> [Turn],
    /// The thread's latest goal and how far it has come.
    "goal/get" => GoalGet { thread: ThreadId } -> [Goal],
    /// Tasks running and recently finished in every project the caller owns, running first,
    /// then newest first.
    "tasks/list" => TasksList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    } -> [Tasks],
    /// Stops the task running in a thread.
    "tasks/stop" => TasksStop { thread: ThreadId } -> [Done],
    /// The commands a thread keeps running in the background, oldest first.
    "command/list" => CommandList { thread: ThreadId } -> [Commands],
    /// Stops one of a thread's commands in the background, and everything it started.
    "command/stop" => CommandStop { thread: ThreadId, command: CommandId } -> [Done],
    /// Runs a prompt, or works toward a goal, as a task at set times, each run in a new thread
    /// working on a copy of its own.
    "schedule/create" => ScheduleCreate {
        /// The project its runs are in, by default as `thread/start` picks one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
        when: When,
        prompt: String,
        #[serde(default, skip_serializing_if = "is_false")]
        goal: bool,
        /// Tokens each run may use. Required: no schedule runs unbounded.
        budget: u64,
    } -> [Schedule],
    /// The schedules of every project the caller owns.
    "schedule/list" => ScheduleList {} -> [Schedules],
    "schedule/delete" => ScheduleDelete { schedule: ScheduleId } -> [Done],
    /// The projects the caller owns.
    "project/list" => ProjectList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
    } -> [Projects],
    "project/get" => ProjectGet { project: ProjectId } -> [Project],
    /// The project the folder at `path` on this computer leads to, as glave run there opens
    /// it: by the folder, else by its repository's `origin`. None, and nothing made, when it
    /// leads to none, or to one the caller does not work in. Only the person on this computer.
    "project/find" => ProjectFind {
        /// The folder's full path.
        path: String,
    } -> [Projects],
    /// A new project, owned by the caller, and its folder, if it has one: its threads work
    /// there. A folder that already leads to a project answers that project, its name as it
    /// was. Only the person on this computer gives a folder.
    "project/create" => ProjectCreate {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<NewFolder>,
    } -> [Project],
    /// Renames a project.
    "project/rename" => ProjectRename { project: ProjectId, name: String } -> [Project],
    /// Gives a project the folder already on this computer at `path`, as its folder's new
    /// place: its threads work there from now on. Only the person on this computer.
    "project/folder/set" => ProjectFolderSet {
        project: ProjectId,
        /// The folder's full path.
        path: String,
    } -> [Project],
    /// Removes a project: its threads go to the archive, their history kept, and it is listed
    /// no more until it is given a folder again. Only the person on this computer.
    "project/remove" => ProjectRemove { project: ProjectId } -> [Done],
    /// The folders inside a folder on this computer, by name, to choose a project's folder
    /// from: the home folder when `path` is absent. Only the person on this computer.
    "folder/list" => FolderList {
        /// A folder's full path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    } -> [Folders],
    /// Whether glave remembers for the caller: theirs to turn on or off, unless their
    /// organization turned memory off or it is not theirs to choose here.
    "memory/get" => MemoryGet {} -> [Remembering],
    /// Turns memory on or off for the caller from their threads' next turn: their own choice,
    /// kept by the server. What they saved stays, theirs to see and change.
    "memory/set" => MemorySet { on: bool } -> [Remembering],
    /// The memories here: the person's and the project's, or one `scope`.
    "memory/list" => MemoryList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<MemoryScope>,
        /// The project whose memory, beside the person's own: the caller's Chats when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
    } -> [Memories],
    "memory/save" => MemorySave {
        text: String,
        scope: MemoryScope,
        /// The project whose memory, beside the person's own: the caller's Chats when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
    } -> [Memories],
    "memory/edit" => MemoryEdit {
        memory: MemoryId,
        text: String,
        /// The project whose memory, beside the person's own: the caller's Chats when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
    } -> [Memories],
    /// Saved in the new scope, forgotten in the old.
    "memory/move" => MemoryMove {
        memory: MemoryId,
        scope: MemoryScope,
        /// The project whose memory, beside the person's own: the caller's Chats when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
    } -> [Memories],
    /// Forgets one memory, or with `all`, every memory of `scope`. Chats keep what was said in
    /// them: the one a memory was saved in goes only by `thread/delete`.
    "memory/forget" => MemoryForget {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory: Option<MemoryId>,
        #[serde(default, skip_serializing_if = "is_false")]
        all: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<MemoryScope>,
        /// The project whose memory, beside the person's own: the caller's Chats when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
    } -> [Memories],
    /// Undoes glave's latest change to a memory, within 30 days.
    "memory/restore" => MemoryRestore {
        memory: MemoryId,
        /// The project whose memory, beside the person's own: the caller's Chats when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project: Option<ProjectId>,
    } -> [Memories],
    /// Connects a model account: the key is checked with the service, then kept sealed, and
    /// the account's models join `model/list`. Connecting the same service (and URL) again
    /// replaces its account. Only the person on this computer.
    "provider/connect" => ProviderConnect {
        kind: ProviderKind,
        /// The account's key, sent once and never answered back.
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        key: Secret,
        /// The server's API, for a `compatible` account: `https`, or `http` on this computer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    } -> [Provider],
    /// The model accounts connected here.
    "provider/list" => ProviderList {} -> [Providers],
    /// Lists every account's models again with its service, now, and answers with the
    /// accounts. Their catalogs are also refreshed daily on their own.
    "provider/refresh" => ProviderRefresh {} -> [Providers],
    /// Removes a model account, destroying the key it was kept with.
    "provider/remove" => ProviderRemove { provider: ProviderId } -> [Done],
    /// Connects a git host: the token is checked with the host where it has a way to, then
    /// kept sealed, and git and the host's own tool sign in with it, asking at each use.
    /// Connecting the same host again replaces it. Only the person on this computer.
    "credential/connect" => CredentialConnect {
        /// Where the host is, such as `https://github.com`: `https`, or `http` on this computer.
        url: String,
        /// The token, sent once and never answered back.
        #[serde(serialize_with = "glave_event::serialize_exposed")]
        token: Secret,
        /// The user name git sends with the token, for a host that needs one; else whom the
        /// host says the token is for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    } -> [Credential],
    /// The git hosts connected here, and what signing in to GitHub in the browser offers.
    "credential/list" => CredentialList {} -> [Credentials],
    /// Removes a git host, destroying its token.
    "credential/remove" => CredentialRemove { credential: CredentialId } -> [Done],
    /// Starts signing in to GitHub in the browser: the code to enter, and where.
    "credential/device/start" => CredentialDeviceStart {} -> [Device],
    /// Waits a little for the sign-in `flow`: the host, connected, once the person approved
    /// it on GitHub; else the flow again, to ask again.
    "credential/device/poll" => CredentialDevicePoll { flow: String } -> [Credential, Waiting],
    /// Starts signing in to a connection, a remote MCP server of the person's settings, as
    /// MCP's authorization says: the page the person opens in the browser, which comes back to this
    /// server. Only the person on this computer.
    "connection/start" => ConnectionStart {
        /// What the person's `mcp.json` calls the server, or its address.
        server: String,
    } -> [SignIn],
    /// Waits a little for the sign-in `flow`: the connection, signed in to, once the person approved
    /// glave in the browser; else the flow again, to ask again.
    "connection/poll" => ConnectionPoll { flow: String } -> [Connection, Waiting],
    /// The connections signed in to here, then the person's remote servers that ask them to
    /// sign in and are not signed in to: each with how much it was used this month, once it
    /// is in the person's settings. Web search is not among them (`search/get`).
    "connection/list" => ConnectionList {} -> [Connections],
    /// Removes a connection's sign-in, destroying its tokens.
    "connection/remove" => ConnectionRemove {
        /// What the person's `mcp.json` calls the server, or its address.
        server: String,
    } -> [Done],
    /// Whether the person's agent may search the web, and, never theirs to choose, why.
    "search/get" => SearchGet {
        /// Also how much it searched this month, which reads the month's threads: for
        /// Settings, not for a client that only shows whether it is on.
        #[serde(default)]
        usage: bool,
    } -> [Search],
    /// Turns web search on or off for the person: on, its server joins their own settings,
    /// `mcp.json` in glave's folder, and their threads may search from the next turn; off, it
    /// leaves them, unless they set it up there themselves. Only the person on this computer.
    "search/set" => SearchSet { on: bool } -> [Search],
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

/// A field that is given, `null` included, as distinct from one left out, which its
/// `#[serde(default)]` reads as `None`: `null` is `Some(None)`.
fn given<'de, D, T>(field: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(field).map(Some)
}

/// What a request may carry besides its params, in `params._meta`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Meta {
    /// For `turn/start`: the same key for the same thread returns the turn already started
    /// instead of starting another, so a client may retry safely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<IdempotencyKey>,
    /// For `turn/start`: the W3C trace the turn belongs to (`00-<trace id>-<parent id>-<flags>`),
    /// recorded with the turn and joined by glave's spans. One that is not one is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
}

impl Meta {
    fn is_empty(&self) -> bool {
        self.idempotency_key.is_none() && self.traceparent.is_none()
    }
}

/// A client's name for one attempt at a turn: 1 to 255 bytes of text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// The longest key, in bytes.
    pub const MAX: usize = 255;

    pub fn new(key: impl Into<String>) -> Result<Self, String> {
        let key = key.into();
        if key.is_empty() || key.len() > Self::MAX {
            return Err(format!(
                "an idempotency key is 1 to {} bytes, got {}",
                Self::MAX,
                key.len()
            ));
        }
        Ok(Self(key))
    }
}

impl TryFrom<String> for IdempotencyKey {
    type Error = String;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl From<IdempotencyKey> for String {
    fn from(key: IdempotencyKey) -> Self {
        key.0
    }
}

/// Which threads `thread/list` shows.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Show {
    /// Threads that are neither archived nor deleted.
    #[default]
    Active,
    Archived,
    /// Deleted threads, waiting for their content to be erased: for the operator that does it.
    Deleted,
}

/// The person's answer to an approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    AllowOnce,
    AllowForThread,
    Deny,
}

/// Why a watch ended (`thread/watch`, `thread/list/watch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchEnd {
    /// The client ended it.
    Unwatched,
    /// The thread was deleted: nothing more will be recorded in it.
    Deleted,
}

/// Something a turn or a watched thread sends while it runs.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    /// A stored event, as it is appended.
    #[serde(rename = "thread/event")]
    Event { thread: ThreadId, event: RawEvent },
    /// Progress, never stored.
    #[serde(rename = "turn/live")]
    Live { thread: ThreadId, update: Update },
    /// A task ended: sent to the thread's watchers. Never stored as such; the ledger keeps it.
    #[serde(rename = "task/finished")]
    TaskFinished { thread: ThreadId, task: TaskInfo },
    /// A thread a list watch follows changed: its summary now, as `thread/list` shows it.
    #[serde(rename = "thread/changed")]
    Changed {
        thread: ThreadId,
        summary: Box<ThreadSummary>,
    },
    /// The watching caller's own settings or connections changed through this server: read
    /// `setting` again. Sent on their list watches.
    #[serde(rename = "settings/changed")]
    SettingsChanged { setting: Setting },
}
