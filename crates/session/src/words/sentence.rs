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
    GoalStopped = "goal-stopped" "stopping the goal stopped",
    NoCommand = "no-command" "that command is not running in this thread",
    CommandNotStopped { reason: String } = "command-not-stopped"
        "the command could not be stopped: { $reason }",

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

    // Models and permissions.
    NoModelYet = "no-model-yet" "no model is set up here yet: connect a model account first",
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

    // Memory.
    MemoryOff = "memory-off" "memory is off for this server",
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
    NoLanguage { tag: String } = "no-language"
        "glave does not write in \"{ $tag }\": choose another language",
}
