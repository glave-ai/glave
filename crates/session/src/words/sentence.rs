//! Every sentence the server writes for a person, in one table: its variant, the values it is
//! written with, its message's id in the catalog, and its English, a Fluent pattern. A
//! translation, once a speaker has read one, gives each id its own words with the same values.

use fluent_bundle::{FluentArgs, FluentValue};

/// Makes [`Sentence`] from the table: each line `Variant { value: Type } = "id" "English"`.
macro_rules! sentences {
    ($(
        $variant:ident $({ $($arg:ident: $kind:ty),+ $(,)? })? = $id:literal $english:literal,
    )*) => {
        /// A sentence of the server's own, with the values it is written with: written in
        /// English where it is made, and in the caller's language where a transport sends it.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub(crate) enum Sentence {
            $($variant $({ $($arg: $kind),+ })?,)*
        }

        impl Sentence {
            /// Each sentence's message id, the names of the values it is written with, and its
            /// English.
            #[cfg(test)]
            pub(crate) const ALL: &[(&str, &[&str], &str)] = &[
                $(($id, &[$($(stringify!($arg)),+)?], $english),)*
            ];

            /// Its message's id in the catalog.
            pub(crate) fn id(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $id,)*
                }
            }

            /// Its English, as a Fluent pattern.
            pub(crate) fn english(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $english,)*
                }
            }

            /// The values it is written with, by name.
            pub(crate) fn args(&self) -> FluentArgs<'_> {
                let mut args = FluentArgs::new();
                match self {
                    $(Self::$variant $({ $($arg),+ })? => {
                        $($(args.set(stringify!($arg), FluentValue::from($arg));)+)?
                    })*
                }
                args
            }
        }
    };
}

sentences! {
    // glave's own records.
    RecordsFailed = "records-failed"
        "glave could not read or save its records: try again, and restart glave if it keeps happening",
    WrittenElsewhere = "written-elsewhere"
        "this changed elsewhere while glave was saving it: look again, then try again",
    NoPerson = "no-person" "this server has no person",
    Stopping = "stopping"
        "glave is stopping, so it starts nothing new; try again once it runs again",
    ServerStopped = "server-stopped" "the server stopped",
    Cancelled = "cancelled" "cancelled",

    // The organization's people.
    AdminsOnly = "admins-only" "only your organization's admins can do this",
    OwnersOnly = "owners-only"
        "only an owner can make someone an owner, or change an owner's role",
    NoSuchPerson { person: String } = "no-such-person"
        "no one with the id { $person } has signed in here",

    // Threads and turns.
    NoThread { thread: String } = "no-thread" "no thread { $thread } in this project",
    TurnRunning { thread: String } = "turn-running"
        "a turn is already running in thread { $thread }",
    TurnWaits { thread: String } = "turn-waits"
        "a turn in thread { $thread } waits for an answer: answer it, or stop the turn, first",
    NoTurn { thread: String } = "no-turn" "no turn is running in thread { $thread }",
    TurnEnded = "turn-ended" "the turn already ended",
    TurnStopped = "turn-stopped" "the turn stopped",
    TurnUnfollowed = "turn-unfollowed"
        "the turn waits for an answer, and this server stopped following it: answer it, and follow the thread to see it go on",
    RunningElsewhere { thread: String } = "running-elsewhere"
        "thread { $thread } is working on another glave server right now: try again once it is done there",
    QueuedElsewhere = "queued-elsewhere"
        "a turn was already running in this thread: your message was queued to it, and the model reads it at its next step",
    NoChoiceWhileRunning = "no-choice-while-running"
        "a model, reasoning level or permissions are chosen for a turn to start with: while a turn runs, send the message without one, or choose once the turn has ended",
    NoComputerYet { computer: String } = "no-computer-yet"
        "threads on a computer of your own (computer { $computer }) are not available yet: start the thread without `computer`",
    NoFolderForWorktree = "no-folder-for-worktree"
        "there is no project folder here to make a worktree of: threads here work in a scratch folder of their own",
    BudgetOnlyInBackground = "budget-only-in-background"
        "turn/start takes a budget only with background: true; for a turn you follow, use a goal",
    ChangeStopped = "change-stopped" "the change stopped",
    RenameStopped = "rename-stopped" "the rename stopped",
    CompactionStopped = "compaction-stopped" "the compaction stopped",
    AnswerNotRecorded = "answer-not-recorded" "the answer was not recorded",
    PageGone = "page-gone"
        "that page of threads can't be found anymore; ask for the first page again",
    NoGoal { thread: String } = "no-goal" "thread { $thread } has no active goal",
    NoPlan { thread: String } = "no-plan"
        "thread { $thread } has no plan waiting for approval: plan mode proposes one",
    NothingBuilt = "nothing-built" "a plan can't be built here: only Plan is allowed",
    PlanStepsInvalid { most: u64, longest: u64 } = "plan-steps-invalid"
        "a plan has 1 to { $most } steps, each a line of at most { $longest } characters",
    GoalInPlan = "goal-in-plan"
        "a goal doesn't run in Plan, which stops at a plan for you to build: choose another mode for it",
    GoalStopped = "goal-stopped" "stopping the goal stopped",
    NoCommand = "no-command" "that command is not running in this thread",
    CommandNotStopped { reason: String } = "command-not-stopped"
        "the command could not be stopped: { $reason }",
    Archived { thread: String } = "archived" "thread { $thread } is archived; unarchive it first",
    NoOpenTurn { thread: String } = "no-open-turn" "no turn is open in thread { $thread }",
    NoApproval { approval: String } = "no-approval" "no approval { $approval } is pending",
    NoQuestion { question: String } = "no-question" "no question { $question } is waiting",
    PromptBlocked { reason: String } = "prompt-blocked" "a hook stopped this prompt: { $reason }",
    PromptFailed { reason: String } = "prompt-failed"
        "the command's text could not be had from its server: { $reason }",
    TitleLength { most: usize } = "title-length" "a title is 1 to { $most } characters, not only spaces",
    QueueFull { most: usize } = "queue-full"
        "{ $most } messages are already waiting for the model to read them: wait until it has, or stop it",
    RecordsBusy = "records-busy" "glave's records kept changing while it wrote to them; try again",

    // Watching.
    WatchEnded = "watch-ended" "the watch already ended",
    WatchStopped = "watch-stopped" "the watch stopped",
    ConnectionClosed = "connection-closed" "the connection closed",
    FollowsTurn { thread: String } = "follows-turn"
        "this connection follows a turn in thread { $thread }: watch it once the turn has answered, or from another connection",
    WatchesThread { thread: String } = "watches-thread"
        "this connection watches thread { $thread } already",
    WatchesProject { project: String } = "watches-project"
        "this connection watches the threads of project { $project } already",
    WatchesYours = "watches-yours" "this connection watches your threads already",
    TooManyWatches { most: usize } = "too-many-watches"
        "a connection watches at most { $most } threads and lists at once: unwatch one first",

    // A model provider's failures: glave's own words, with what the provider said as it said it.
    ProviderHttp { status: String, body: String } = "provider-http"
        "the model provider returned HTTP { $status }: { $body }",
    ProviderCredentials { reason: String } = "provider-credentials"
        "could not get credentials for the model provider: { $reason }",
    ProviderUnreachable { reason: String } = "provider-unreachable"
        "could not reach the model provider: { $reason }",
    ProviderMalformed { reason: String } = "provider-malformed"
        "the model provider sent a malformed response: { $reason }",
    ProviderNoOutput { reason: String } = "provider-no-output"
        "the model provider ended the response in an error before any output: { $reason }",
    ProviderBrokeOff = "provider-broke-off" "the model provider's answer broke off partway:{ \" \" }",
    ProviderSilent { seconds: String } = "provider-silent"
        "the model provider sent nothing for { $seconds } s",
    NoModelServes { reason: String } = "no-model-serves" "no model could serve the request: { $reason }",
    OverBudget { budget: String, detail: String } = "over-budget"
        "the { $budget } budget would be exceeded: { $detail }",

    // Models and permissions.
    NoModelYet = "no-model-yet" "no model is set up here yet: connect a model account first",
    ModelIdLength { most: usize } = "model-id-length" "a model id is 1 to { $most } bytes, not only spaces",
    EffortWord { most: usize } = "effort-word"
        "a reasoning level is one word of 1 to { $most } letters, such as high",
    ModelNotRun { model: String, models: String } = "model-not-run"
        "glave does not run the model \"{ $model }\" here; it runs { $models }",
    NoEffortChoice { model: String } = "no-effort-choice"
        "{ $model } offers no choice of how much it reasons: start without a level",
    NoSuchEffort { model: String, effort: String, efforts: String } = "no-such-effort"
        "{ $model } does not offer the reasoning level \"{ $effort }\": choose one of { $efforts }",
    PermissionsNotAllowed { mode: String, left: String } = "permissions-not-allowed"
        "{ $mode } isn't allowed here: choose { $left }",
    Unsandboxed = "unsandboxed"
        "This computer can't run the sandbox, so Auto and Ask first both ask before every change and command.",
    OnlyReadOnlySandbox = "only-read-only-sandbox"
        "This computer can only run the sandbox that keeps files as they are, so Auto asks before every change and command.",

    // Files sent with a message.
    FileName { most: usize } = "file-name" "a file's name is 1 to { $most } characters",
    NoVideos = "no-videos" "glave can't use videos yet: add a screenshot or a few images instead",
    FileUnreadable = "file-unreadable" "the file's content could not be read: add it again",
    TooManyFiles { most: usize } = "too-many-files" "a message holds at most { $most } files",
    NoImages = "no-images"
        "this model can't read images or PDFs: choose one that can, or remove the file",
    FileGone { name: String } = "file-gone"
        "\"{ $name }\" is no longer here: add it to the message again",
    FileKind = "file-kind"
        "glave can't read this kind of file yet: add an image (PNG, JPEG, GIF or WebP), a PDF or a text file",
    ImageUnreadable { name: String } = "image-unreadable"
        "\"{ $name }\" can't be opened as an image: add it again, or a screenshot of it",
    ImageTooLarge { name: String, pixels: u32 } = "image-too-large"
        "\"{ $name }\" is too large to open, at most { $pixels } pixels on a side: add a smaller copy, or a screenshot of part of it",
    FileTooLarge { megabytes: u64 } = "file-too-large"
        "this file is larger than { $megabytes } MB: add a smaller one, or the part that matters",
    FilesNotForTasks = "files-not-for-tasks"
        "files are added to a message someone follows, not to a task: send it without background",
    SentTooSlowly { minutes: u64 } = "sent-too-slowly"
        "what you sent did not arrive whole within { $minutes } minutes: send it again, on a steadier connection",

    // Projects and their folders.
    NoProject { project: String } = "no-project" "no project { $project } here",
    FolderGone = "folder-gone" "This project's folder isn't there anymore.",
    FolderStillThere = "folder-still-there" "this project's folder is still where it was",
    FolderOthers = "folder-others" "that folder is already another project's",
    ProjectBusy = "project-busy"
        "something is running in this project: stop it first, then remove the project",
    OnlyProjects = "only-projects" "only a project has a folder, and can be removed",
    SetFolderOnlyHere = "set-folder-only-here"
        "only the person on the computer glave runs on gives a project its folder",
    RemoveOnlyHere = "remove-only-here"
        "only the person on the computer glave runs on removes a project",
    NewFolderOnlyHere = "new-folder-only-here"
        "a project's folder is on the computer glave runs on, so only the person there can give one",
    LookOnlyHere = "look-only-here"
        "a project's folder is on the computer glave runs on, so only the person there can look for one",
    FoldersOnlyHere = "folders-only-here"
        "folders are shown only to the person on the computer glave runs on",
    FolderElsewhere = "folder-elsewhere"
        "this project's folder is on another computer, which glave cannot reach from here yet",
    FolderNotYours { folder: String } = "folder-not-yours"
        "{ $folder } belongs to a project you do not work in",
    TheFullPath { path: String } = "the-full-path" "give the folder's full path, not \"{ $path }\"",
    AFullPath { path: String } = "a-full-path" "give a folder's full path, not \"{ $path }\"",
    NoHome = "no-home"
        "this computer has no home folder to start from; give a folder's full path",
    NoFolderAt { path: String } = "no-folder-at" "there is no folder at { $path }",
    HomeHoldsChats = "home-holds-chats"
        "your home folder holds your Chats, so it is no project's folder: pick a folder inside it",
    AboveHome = "above-home"
        "this folder holds your home folder, so it is no project's folder: pick a folder inside your home folder, or another",
    TemporaryFolder = "temporary-folder"
        "this is the computer's temporary folder, which it empties, so it is no project's folder: pick another folder",
    FolderNotMade { reason: String } = "folder-not-made" "could not make the folder: { $reason }",
    FolderNotOpened { path: String, reason: String } = "folder-not-opened"
        "could not open the folder at { $path }: { $reason }",
    CloneFailed { url: String, reason: String } = "clone-failed"
        "could not clone { $url }: { $reason }. If the repository is private, connect its git host first",
    NotARepository { url: String } = "not-a-repository"
        "\"{ $url }\" is not a repository's address: give its https:// address, such as https://github.com/acme/app, with no user name or token in it",

    // What an approval asks, by what would run: recorded in English in `approval.requested`,
    // by which its English is found again here.
    AskCommand = "ask-command" "Run this command?",
    AskEdit = "ask-edit" "Edit this file?",
    AskWrite = "ask-write" "Write this file?",
    AskRead = "ask-read" "Read this file?",
    AskPage = "ask-page" "Open this page?",
    AskAllow = "ask-allow" "Allow this?",

    // Project trust.
    NothingToTrust = "nothing-to-trust" "nothing in this thread's project waits to be trusted",
    TrustChanged = "trust-changed"
        "this project changed since you were asked: look again at what it brings now",
    TrustOnlyHere = "trust-only-here"
        "only the person on this computer can trust a project: what it brings runs on this computer",

    // Tasks and schedules.
    TaskInPlace = "task-in-place"
        "a task always works on a copy of its project's folder, never in the folder itself: leave the worktree choice out, or turn it on",
    TaskNoCopy = "task-no-copy"
        "a task works on a copy of its project's folder made with git, and this project's folder is not a git repository with a commit yet: make it one, with a first commit, to run tasks and schedules in it",
    TaskThreadInFolder = "task-thread-in-folder"
        "this thread works in its project's folder itself, and a task always works on a copy: start the task in a new thread, which gets a copy of its own",
    TooManyTasks { running: usize, most: usize } = "too-many-tasks"
        "{ $running } tasks are running, the most policy allows ([tasks] max_running = { $most }); stop one first",
    TaskStopped = "task-stopped" "the task stopped",
    NoTask { thread: String } = "no-task" "no task is running in thread { $thread } in this server",
    CronFields { text: String } = "cron-fields"
        "a cron expression has five fields (minute hour day month weekday), got \"{ $text }\"",
    CronField { field: String, text: String, low: u8, high: u8 } = "cron-field"
        "the { $field } field \"{ $text }\" is not valid: { $low } to { $high }, *, a-b, */n",
    ScheduleNoBudget = "schedule-no-budget"
        "a schedule's budget is the tokens each run may use, more than 0",
    ScheduleNoPrompt = "schedule-no-prompt" "a schedule needs a prompt",
    TooManySchedules { kept: usize, most: usize } = "too-many-schedules"
        "{ $kept } schedules are kept, the most policy allows ([tasks] max_schedules = { $most }); delete one first",
    ScheduleNotKept = "schedule-not-kept" "the schedule was not kept",
    NoSchedule { schedule: String } = "no-schedule" "no schedule { $schedule } here",

    // What replies drew on.
    Sources { pages: u64 } = "sources"
        "Sources: { $pages ->\n    [1] 1 page\n   *[other] { $pages } pages\n }",

    // What a thread changed in files.
    ChangesLeftOut = "changes-left-out"
        "Changes made by commands or other tools are not listed.",

    // A thread's plan.
    PlanProposed { steps: u64 } = "plan-proposed"
        "Proposed plan: { $steps ->\n    [1] 1 step\n   *[other] { $steps } steps\n }",
    PlanProgress { done: u64, steps: u64 } = "plan-progress"
        "Plan: { $done } of { $steps ->\n    [1] 1 step\n   *[other] { $steps } steps\n } done",

    // A run of tool calls, in a line ("Ran 2 commands, read 1 file (1 failed)"): a clause for
    // each kind of call, `how` saying whether it comes first and whether a call still runs
    // (`first-now`, `then-now`, `first-done`, `then-done`), then how many failed.
    RanCommands { how: String, count: u64 } = "ran-commands"
        "{ $how ->\n    [first-now] Running\n    [then-now] {\", \"}running\n    [first-done] Ran\n   *[then-done] {\", \"}ran\n } { $count ->\n    [1] 1 command\n   *[other] { $count } commands\n }",
    ReadFiles { how: String, count: u64 } = "read-files"
        "{ $how ->\n    [first-now] Reading\n    [then-now] {\", \"}reading\n    [first-done] Read\n   *[then-done] {\", \"}read\n } { $count ->\n    [1] 1 file\n   *[other] { $count } files\n }",
    RanSearches { how: String, count: u64 } = "ran-searches"
        "{ $how ->\n    [first-now] Running\n    [then-now] {\", \"}running\n    [first-done] Ran\n   *[then-done] {\", \"}ran\n } { $count ->\n    [1] 1 search\n   *[other] { $count } searches\n }",
    EditedFiles { how: String, count: u64 } = "edited-files"
        "{ $how ->\n    [first-now] Editing\n    [then-now] {\", \"}editing\n    [first-done] Edited\n   *[then-done] {\", \"}edited\n } { $count ->\n    [1] 1 file\n   *[other] { $count } files\n }",
    UsedWeb { how: String, count: u64 } = "used-web"
        "{ $how ->\n    [first-now] Using the web\n    [then-now] {\", \"}using the web\n    [first-done] Used the web\n   *[then-done] {\", \"}used the web\n } { $count ->\n    [1] once\n   *[other] { $count } times\n }",
    UsedTools { how: String, count: u64 } = "used-tools"
        "{ $how ->\n    [first-now] Using\n    [then-now] {\", \"}using\n    [first-done] Used\n   *[then-done] {\", \"}used\n } { $count ->\n    [1] 1 tool\n   *[other] { $count } tools\n }",
    CallsFailed { count: u64 } = "calls-failed" "{\" \"}({ $count } failed)",

    // Memory.
    MemoryOff = "memory-off" "memory is off for this server",
    MemoryOnItsComputer = "memory-on-its-computer"
        "memory is turned on or off on the computer glave runs on, by its person",
    ForgetWhat = "forget-what" "memory/forget takes `memory`, or `all: true` with a `scope`",

    // Accounts, sign-in and the person's own settings.
    ModelsNotHere = "models-not-here"
        "this server's models are set up by whoever runs it, not from here",
    ModelAccountsOnItsComputer = "model-accounts-on-its-computer"
        "model accounts are connected on the computer glave runs on, by its person",
    AccountsNotHere = "accounts-not-here"
        "connecting accounts is set up by whoever runs this server, not from here",
    AccountsOnItsComputer = "accounts-on-its-computer"
        "accounts are connected on the computer glave runs on, by its person",
    ConnectionsNotHere = "connections-not-here"
        "signing in to connections is set up by whoever runs this server, not from here",
    ConnectionsOnItsComputer = "connections-on-its-computer"
        "connections are signed in to on the computer glave runs on, by its person",
    SignInNeedsServer = "sign-in-needs-server"
        "signing in to a connection needs glave running in the background, for your browser to come back to: open glave, then try again",
    SignInNoWayBack = "sign-in-no-way-back"
        "this glave has no address your browser can come back to after signing in: whoever runs it gives it the address people reach it at, then try again",
    SearchDetail = "search-detail" "Uses Exa, free. Your searches go to Exa.",
    SearchNotHere = "search-not-here"
        "web search is set up by whoever runs this server, not from here",
    SearchOnItsComputer = "search-on-its-computer"
        "web search is turned on or off on the computer glave runs on, by its person",    CallsThisMonth { calls: u64 } = "calls-this-month"
        "{ $calls ->\n    [0] No calls\n    [1] One call\n   *[other] { $calls } calls\n } this month",
    SearchesThisMonth { searches: u64 } = "searches-this-month"
        "{ $searches ->\n    [0] No searches\n    [1] One search\n   *[other] { $searches } searches\n } this month",
    BrowserNotKept = "browser-not-kept" "glave could not keep this browser's sign-in: try again",
    PasskeysOnlyTheirs = "passkeys-only-theirs"
        "only the person this glave is for changes its passkeys",
    NotABrowser = "not-a-browser" "this is not a browser's sign-in, so there is none to end",
    SignOutOnlyHere = "sign-out-only-here"
        "only the person on this computer signs its browsers out",
    NoBrowsers = "no-browsers" "this server signs no browser in: open glave on its computer",
    LinksOnlyHere = "links-only-here" "only glave on this computer makes sign-in links",
    AwakeOnServer = "awake-on-server"
        "this glave is a server, which doesn't sleep: there is nothing to keep awake",
    AwakeOnlyHere = "awake-only-here"
        "only the person on this computer chooses whether it stays awake",
    AwakeNeedsSystemd = "awake-needs-systemd"
        "This computer can't be kept awake while glave works: that needs systemd, which it doesn't run.",
    AwakeWithNothing = "awake-with-nothing"
        "This computer can't be kept awake while glave works: its system has nothing for it.",

    // Signing a browser in.
    LinkSpent = "link-spent"
        "that sign-in link was used already or has expired: open glave again for a new one",
    SetupSpent = "setup-spent"
        "that link to make a passkey was used already or has expired: on the computer glave runs on, ask glave for a new one",
    TooManyLinks = "too-many-links"
        "too many sign-in links that do not work were tried: wait a minute, then try again",
    SignInEnded = "sign-in-ended" "this browser's sign-in has ended: open glave again to sign in",
    SignedInThere = "signed-in-there"
        "this browser was signed in on the computer glave runs on, and signs in only there",
    NoPasskeysHere = "no-passkeys-here"
        "this glave makes no passkeys for this page: open glave on its computer to sign in",
    NoPasskeyYet = "no-passkey-yet"
        "no passkey signs in to this glave yet: open a link glave gives you, and make one",
    PasskeyExpired = "passkey-expired" "that passkey request has expired: try again",
    PasskeyUnknown = "passkey-unknown"
        "that passkey no longer signs in to this glave: sign in with another, or open a link glave gives you to make a new one",
    PasskeyRefused { reason: String } = "passkey-refused" "the passkey was not accepted: { $reason }",

    // Signing in with a token.
    NoSignIn = "no-sign-in"
        "no sign-in: send an OIDC access token, or from this computer the local token in serve.token, as `Authorization: Bearer <token>`",
    TokenMalformed { reason: String } = "token-malformed"
        "the token is not a JWT signed with RS256 or ES256: { $reason }",
    UnknownIssuer = "unknown-issuer" "the token's issuer is not one this server trusts",
    IssuerKeys { issuer: String, reason: String } = "issuer-keys"
        "could not get the signing keys of { $issuer }: { $reason }",
    UnknownKey { issuer: String } = "unknown-key" "the token was not signed by a key { $issuer } publishes",
    TokenInvalid { reason: String } = "token-invalid" "the token is not valid: { $reason }",
    TokenExpired = "token-expired" "the token has expired",
    TokenNotYet = "token-not-yet" "the token is not valid yet",
    IssuerInsecure { issuer: String } = "issuer-insecure"
        "the issuer \"{ $issuer }\" is not an https URL (http is allowed only on this computer)",
    NoIssuers = "no-issuers" "no issuer is trusted: a network listener never runs without sign-in",
    KeysClient { reason: String } = "keys-client"
        "could not make the HTTP client that fetches keys: { $reason }",
    LocalTokenHereOnly = "local-token-here-only" "the local token is accepted only from this computer",
    WrongLocalToken = "wrong-local-token"
        "that is not this server's local token: read it again from serve.token",
    SignedInThereOnly = "signed-in-there-only"
        "this browser was signed in on the computer glave runs on, and its sign-in works only there: sign in with a passkey here",
    NoLanguage { tag: String } = "no-language"
        "glave does not write in \"{ $tag }\": choose another language",
}

impl Sentence {
    /// The sentence an approval asks with, by the English its `approval.requested` recorded:
    /// none for words the table does not hold, such as a wording from before.
    pub(crate) fn asking(english: &str) -> Option<Self> {
        [
            Self::AskCommand,
            Self::AskEdit,
            Self::AskWrite,
            Self::AskRead,
            Self::AskPage,
            Self::AskAllow,
        ]
        .into_iter()
        .find(|sentence| sentence.english() == english)
    }
}
