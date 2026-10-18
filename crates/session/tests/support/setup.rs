//! A session server for a test, built from a `Setup` over a real temporary folder, and a log
//! two servers can share.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_computer::LocalComputers;
use glave_event::{
    At, Checkout, EventId, FolderSource, Header, Principal, RawEvent, Seq, SpaceFolderSet,
    ThreadId, encode,
};
use glave_harness::{ApprovalMode, Memory, Projects, Settings, Workspace};
use glave_keys::{KeyFileRoot, KeySettings};
use glave_ports::testing::{CountingEntropy, ManualClock};
use glave_ports::{
    Completion, ComputerId, EventLog, GitCredentials, LogError, MemoryBlobs, MemoryLog,
    MemorySpaces, OrgId, Scope, Space, SpaceId, SpaceKind, Spaces, WorkspaceInfo,
};
use glave_session::protocol::ModelInfo;
use glave_session::signin::Browsers;
use glave_session::{
    Answering, GitHosts, ProjectTrust, Providers, Server, TaskLimits, Wiring, space_ledger,
};
use glave_store_sqlite::Store;
use tokio::sync::Notify;

use super::script::{Script, Seen};

/// A test server on the log `L`.
pub type Tested<L = MemoryLog> = Server<Script, L, MemoryBlobs>;

/// A fresh folder for the test called `name`.
pub fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The SQLite store at `database`, sealed under a key file beside it, so opening it again
/// reads what was written before.
pub async fn sqlite(database: &Path) -> Store {
    let root = KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
    Store::open(database, KeySettings::new(Arc::new(root)))
        .await
        .unwrap()
}

/// The organization and space every test server serves.
pub fn scope() -> Scope {
    Scope {
        org: OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        space: SpaceId::from_uuid(uuid::Uuid::from_u128(2)),
    }
}

/// What a test server is built from.
pub struct Setup {
    pub answers: Vec<Completion>,
    /// Each answer waits here until the test lets it through.
    pub gate: Option<Arc<Notify>>,
    pub answering: Answering,
    pub settings: Settings,
    /// Ticks a millisecond a reading unless the test sets one it moves itself.
    pub clock: ManualClock,
    /// Where the ids start: a second server on the same log takes another, as another
    /// process draws other ids.
    pub seed: u64,
    pub limits: TaskLimits,
    /// Models offered beside the settings' own.
    pub models: Vec<ModelInfo>,
    /// The approval modes a thread may choose.
    pub modes: Vec<ApprovalMode>,
    pub memory: Option<Arc<dyn Memory>>,
    /// Where worktrees are kept; with none, a thread asking for one is refused.
    pub worktrees: Option<PathBuf>,
    /// Where model accounts are kept; with none, connecting one is refused.
    pub providers: Option<Arc<dyn Providers>>,
    /// Where accounts with hosts are kept; with none, connecting one is refused.
    pub hosts: Option<Arc<dyn GitHosts>>,
    /// Where new projects' folders are made; with none, making one is refused.
    pub projects: Option<PathBuf>,
    /// Where threads in a folder of their own read their opening instructions, and what their
    /// project turns on.
    pub project_host: Option<Arc<dyn Projects>>,
    /// Where what a project asks to be trusted is read and kept.
    pub trust: Option<Arc<dyn ProjectTrust>>,
    /// Where commands' git, and a new project's clone, ask for credentials.
    pub credentials: Option<Arc<dyn GitCredentials>>,
    /// The spaces another server keeps, to share its store; spaces of its own when none.
    pub spaces: Option<Arc<dyn Spaces>>,
    /// The browsers it signs in; with none, it signs in none.
    pub browsers: Option<Arc<Browsers>>,
}

impl Setup {
    /// A server answering `answers` in the product's defaults, with clients to ask.
    pub fn new(answers: Vec<Completion>) -> Self {
        Self {
            answers,
            gate: None,
            answering: Answering::Clients,
            settings: Settings::new("test-model", "script"),
            clock: ManualClock::ticking(1_791_001_000_000),
            seed: 1,
            limits: TaskLimits::default(),
            models: Vec::new(),
            modes: vec![ApprovalMode::Ask, ApprovalMode::Auto],
            memory: None,
            worktrees: None,
            providers: None,
            hosts: None,
            projects: None,
            project_host: None,
            trust: None,
            credentials: None,
            spaces: None,
            browsers: None,
        }
    }

    /// A server answering `answers` in ask mode, so commands ask, with `answering` to ask.
    pub fn asking(answers: Vec<Completion>, answering: Answering) -> Self {
        let mut setup = Self::new(answers);
        setup.answering = answering;
        setup.settings.approval = ApprovalMode::Ask;
        setup
    }

    /// The server on a log of its own; see `build_on`.
    pub async fn build(self, dir: &Path) -> (Tested, Seen) {
        self.build_on(dir, MemoryLog::new()).await
    }

    /// The server on `log`, its own space a project whose folder is a repository made in the
    /// fresh folder `name`, with worktrees kept beside it, so its tasks work on copies of it;
    /// and that repository.
    pub async fn build_in_repository<L: EventLog + 'static>(
        mut self,
        name: &str,
        log: L,
    ) -> (Tested<L>, Seen, PathBuf) {
        let dir = folder(name);
        let repo = super::git::repository(&dir, "app");
        self.worktrees = Some(dir.join("worktrees"));
        let (server, seen) = self.build_on(&repo, log).await;
        (server, seen, repo)
    }

    /// The server on `log`, its own space a project whose folder is `dir`, and every request
    /// its model receives. Threads in spaces with no folder get scratch folders beside `dir`.
    pub async fn build_on<L: EventLog + 'static>(self, dir: &Path, log: L) -> (Tested<L>, Seen) {
        let (provider, seen) = Script::new(self.answers, self.gate);
        let computers = LocalComputers::for_here(std::env::vars_os())
            .with_scratch(dir.with_extension("scratch"));
        let computers = match &self.worktrees {
            Some(base) => computers.with_worktrees(base),
            None => computers,
        };
        let computers = match &self.projects {
            Some(base) => computers.with_projects(base),
            None => computers,
        };
        // One computer for every test server, as this machine's own id is kept across runs.
        let computers = computers.known_as(ComputerId::from_uuid(uuid::Uuid::from_u128(7)));
        let (computers, info) = computers.open_here(dir).await.unwrap();
        keep_folder(&log, &info).await;
        let workspace = Workspace::new(computers, info);
        let workspace = match self.credentials {
            Some(credentials) => workspace.with_credentials(credentials),
            None => workspace,
        };
        let wiring = Wiring {
            provider,
            log,
            blobs: MemoryBlobs::new(),
            clock: Box::new(self.clock),
            entropy: Box::new(CountingEntropy::starting_at(self.seed << 32)),
            workspace,
            web: None,
            tools: Vec::new(),
            own: Vec::new(),
            worktree: false,
            hooks: glave_harness::Hooks::default(),
            workers: None,
            tasks: self.limits,
            memory: self.memory,
            spaces: self
                .spaces
                .unwrap_or_else(|| Arc::new(MemorySpaces::new()) as Arc<dyn Spaces>),
            models: self.models,
            modes: self.modes,
            providers: self.providers,
            rereads: None,
            git_hosts: self.hosts,
            projects: self.project_host,
            trust: self.trust,
            browsers: self.browsers,
        };
        let server = Server::new(wiring, self.settings, scope(), self.answering);
        // The server's own space, kept as its person's project at a fixed time without reading
        // the clock, so nothing a test checks depends on when it was first asked for.
        let at = At::try_from("2026-10-01T00:00:00.000Z".to_owned()).unwrap();
        let home = Space {
            id: scope().space,
            kind: SpaceKind::Project,
            name: "test".into(),
            owner: Principal::person(server.person().unwrap()),
            members: Vec::new(),
            created_at: at,
            updated_at: at,
        };
        // A second server on another's spaces finds it there already.
        if server
            .spaces()
            .get(scope().org, home.id)
            .await
            .unwrap()
            .is_none()
        {
            server.spaces().create(scope().org, &home).await.unwrap();
        }
        (server, seen)
    }
}

/// Keeps the folder `info` opened as the server's own project's, as glave keeps the folder it
/// runs in: at a fixed time with fixed ids, for the same reason as the space itself. A second
/// server on a shared log finds it there already.
async fn keep_folder<L: EventLog>(log: &L, info: &WorkspaceInfo) {
    let at = At::try_from("2026-10-01T00:00:00.000Z".to_owned()).unwrap();
    let header = Header {
        id: EventId::new(at.timestamp(), [9; 10]).unwrap(),
        thread: space_ledger().unwrap(),
        seq: Seq::FIRST,
        at,
    };
    let set = SpaceFolderSet {
        folder: uuid::Uuid::from_u128(3),
        source: FolderSource::Local,
        checkout: Checkout {
            computer: info.computer.as_uuid(),
            path: info.root.clone(),
        },
        by: None,
    };
    let _ = log.append(&scope(), &[encode(header, &set).unwrap()]).await;
}

/// The server of `Setup::asking`.
pub async fn server(dir: &Path, answers: Vec<Completion>, answering: Answering) -> Tested {
    Setup::asking(answers, answering).build(dir).await.0
}

/// A log in memory that two servers can share, as two processes share a database.
#[derive(Clone, Default)]
pub struct Shared(pub Arc<MemoryLog>);

impl EventLog for Shared {
    fn append(
        &self,
        scope: &Scope,
        events: &[RawEvent],
    ) -> impl Future<Output = Result<(), LogError>> + Send {
        self.0.append(scope, events)
    }

    fn read(
        &self,
        scope: &Scope,
        thread: ThreadId,
        after: Option<Seq>,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<RawEvent>, LogError>> + Send {
        self.0.read(scope, thread, after, limit)
    }

    fn search(
        &self,
        scope: &Scope,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<glave_ports::search::Found>, LogError>> + Send {
        self.0.search(scope, query, limit)
    }

    fn locate(
        &self,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Scope>, LogError>> + Send {
        self.0.locate(thread)
    }

    fn thread_views(
        &self,
        scope: &Scope,
        query: &glave_ports::ThreadQuery,
    ) -> impl Future<Output = Result<Vec<glave_ports::ThreadView>, LogError>> + Send {
        self.0.thread_views(scope, query)
    }

    fn thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<glave_ports::ThreadView>, LogError>> + Send {
        self.0.thread_view(scope, thread)
    }

    fn rebuild_thread_view(
        &self,
        scope: &Scope,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<glave_ports::ThreadView>, LogError>> + Send {
        self.0.rebuild_thread_view(scope, thread)
    }
}
