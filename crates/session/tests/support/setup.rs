//! A session server for a test, built from a `Setup` over a real temporary folder, and a log
//! two servers can share.

use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_computer::LocalComputers;
use glave_event::{
    At, Checkout, EventId, FolderSource, Header, Money, Principal, ProjectFolderSet, RawEvent, Seq,
    ThreadId, encode,
};
use glave_gateway::Credits;
use glave_harness::{Homes, Permissions, ProjectFolders, Settings, Workspace};
use glave_keys::{KeyFileRoot, KeySettings};
use glave_ports::computer::{ComputerId, GitCredentials, WorkspaceInfo};
use glave_ports::memories::Memory;
use glave_ports::testing::{CountingEntropy, ManualClock, TempDir};
use glave_ports::{
    Completion, EventLog, InMemoryBlobs, InMemoryLeases, InMemoryLog, InMemoryProjects, Leases,
    Located, LogError, OrgId, Project, ProjectId, ProjectKind, Projects, Scope, ToolSource,
};
use glave_session::protocol::ModelInfo;
use glave_session::signin::Browsers;
use glave_session::{
    Answering, Connections, GitHosts, KeepAwake, OnWorkers, ProjectTrust, Providers, Server,
    TaskLimits, Wiring, Worktrees,
};
use glave_store_sqlite::Store;
use tokio::sync::Notify;

use super::script::{ScriptedModel, Seen};

/// A test server on the log `L`.
pub(crate) type Tested<L = InMemoryLog> = Server<ScriptedModel, L, InMemoryBlobs>;

/// A fresh folder for the test called `name`, removed when it drops.
pub(crate) fn folder(name: &str) -> TempDir {
    TempDir::new(&format!("session-{name}")).unwrap()
}

/// The SQLite store at `database`, sealed under a key file beside it, so opening it again
/// reads what was written before; its blobs' bytes in memory.
pub(crate) async fn sqlite(database: &Path) -> Store {
    let root = KeyFileRoot::open_or_create(database.with_extension("key")).unwrap();
    let objects = Arc::new(glave_ports::InMemoryObjects::new());
    Store::open(database, KeySettings::new(Arc::new(root)), objects)
        .await
        .unwrap()
}

/// The organization and project every test server serves.
pub(crate) fn scope() -> Scope {
    Scope::glaves_own(
        OrgId::from_uuid(uuid::Uuid::from_u128(1)),
        ProjectId::from_uuid(uuid::Uuid::from_u128(2)),
    )
}

/// What a test server is built from.
pub(crate) struct Setup {
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
    pub memory: Option<Arc<dyn Memory>>,
    /// Where worktrees are kept; with none, a thread asking for one is refused.
    pub worktrees: Option<PathBuf>,
    /// Where model accounts are kept; with none, connecting one is refused.
    pub providers: Option<Arc<dyn Providers>>,
    /// Where accounts with hosts are kept; with none, connecting one is refused.
    pub hosts: Option<Arc<dyn GitHosts>>,
    /// Where sign-ins to connections are kept; with none, signing in is refused.
    pub connections: Option<Arc<dyn Connections>>,
    /// Where new projects' folders are made; with none, making one is refused.
    pub project_base: Option<PathBuf>,
    /// Where threads in a folder of their own read their opening instructions, and what their
    /// project turns on.
    pub project_folders: Option<Arc<dyn ProjectFolders>>,
    /// Where what a project asks to be trusted is read and kept.
    pub trust: Option<Arc<dyn ProjectTrust>>,
    /// Where commands' git, and a new project's clone, ask for credentials.
    pub credentials: Option<Arc<dyn GitCredentials>>,
    /// The projects another server keeps, to share its store; projects of its own when none.
    pub projects: Option<Arc<dyn Projects>>,
    /// The browsers it signs in; with none, it signs in none.
    pub browsers: Option<Arc<Browsers>>,
    /// The glave accounts it keeps; with none, it keeps none.
    pub accounts: Option<Arc<glave_session::signin::Accounts>>,
    /// Where its email goes; with none, it sends none.
    pub mail: Option<Arc<dyn glave_ports::Mailer>>,
    /// Where people reach it from elsewhere; with none, only on its own computer.
    pub origin: Option<glave_ports::host::PublicOrigin>,
    /// Whether it takes sign-ups.
    pub signup: bool,
    /// How many addresses the proxies in front of it add to `X-Forwarded-For`, if it says.
    pub forwarded: Option<std::num::NonZeroU8>,
    /// What keeps its computer awake while work runs; with none, nothing does.
    pub awake: Option<Arc<dyn KeepAwake>>,
    /// What lets go of a thread's worktrees once it is put away; with none, nothing does.
    pub release: Option<Arc<dyn Worktrees>>,
    /// Its organization and its own project: `scope()` unless a test serves another.
    pub home: Scope,
    /// The leases another server keeps, to share its store; its own in memory when none.
    pub leases: Option<Arc<dyn Leases>>,
    /// The model it asks, in place of one answering `answers` in turn.
    pub model: Option<ScriptedModel>,
    /// Tools beside the built-in ones.
    pub tools: Vec<Arc<dyn ToolSource>>,
    /// Variables its computer gives in place of this process's, such as its `HOME`.
    pub variables: Vec<(String, PathBuf)>,
    /// The workers threads of one project work on; with none, every thread works here.
    pub workers: Option<OnWorkers>,
    /// Whether people sign in through an identity provider; with none, it works for one person.
    pub identity_provider: bool,
    /// Each person's own computer; with none, every thread works here.
    pub homes: Option<Arc<dyn Homes>>,
}

impl Setup {
    /// A server answering `answers` in the product's defaults, with clients to ask.
    pub(crate) fn new(answers: Vec<Completion>) -> Self {
        Self {
            answers,
            gate: None,
            answering: Answering::Clients,
            settings: full_access(),
            clock: ManualClock::ticking(1_791_001_000_000),
            seed: 1,
            limits: TaskLimits::default(),
            models: Vec::new(),
            memory: None,
            worktrees: None,
            providers: None,
            hosts: None,
            connections: None,
            project_base: None,
            project_folders: None,
            trust: None,
            credentials: None,
            projects: None,
            browsers: None,
            accounts: None,
            mail: None,
            origin: None,
            signup: false,
            forwarded: None,
            awake: None,
            release: None,
            home: scope(),
            leases: None,
            model: None,
            tools: Vec::new(),
            variables: Vec::new(),
            workers: None,
            identity_provider: false,
            homes: None,
        }
    }

    /// A server answering `answers` in Ask first, so changes and commands ask, with
    /// `answering` to ask.
    pub(crate) fn asking(answers: Vec<Completion>, answering: Answering) -> Self {
        let mut setup = Self::new(answers);
        setup.answering = answering;
        setup.settings.permissions = Permissions::Ask;
        setup
    }

    /// The server on a log of its own; see `build_on`.
    pub(crate) async fn build(self, dir: &Path) -> (Tested, Seen) {
        self.build_on(dir, InMemoryLog::new()).await
    }

    /// The server on `log`, its own project a project whose folder is a repository made in the
    /// fresh folder `name`, with worktrees kept beside it, so its tasks work on copies of it;
    /// that repository; and the folder, removed when it drops.
    pub(crate) async fn build_in_repository<L: EventLog + 'static>(
        mut self,
        name: &str,
        log: L,
    ) -> (Tested<L>, Seen, PathBuf, TempDir) {
        let dir = folder(name);
        let repo = super::git::repository(&dir, "app");
        self.worktrees = Some(dir.join("worktrees"));
        let (server, seen) = self.build_on(&repo, log).await;
        (server, seen, repo, dir)
    }

    /// The server on `log`, its own project a project whose folder is `dir`, and every request
    /// its model receives. Threads in projects with no folder get scratch folders beside `dir`.
    pub(crate) async fn build_on<L: EventLog + 'static>(
        self,
        dir: &Path,
        log: L,
    ) -> (Tested<L>, Seen) {
        self.build_charging(dir, log, None).await
    }

    /// The server of `build_on` on `log`, keeping organizations' credits there, each limited
    /// to `limit` a month until its Owner sets another.
    pub(crate) async fn build_with_credits<L: EventLog + Clone + 'static>(
        self,
        dir: &Path,
        log: L,
        limit: Option<Money>,
    ) -> (Tested<L>, Seen) {
        let credits = Credits::new(
            log.clone(),
            Box::new(self.clock.clone()),
            Box::new(CountingEntropy::starting_at((self.seed << 32) | (1 << 31))),
            limit,
        );
        self.build_charging(dir, log, Some(Arc::new(credits))).await
    }

    async fn build_charging<L: EventLog + 'static>(
        self,
        dir: &Path,
        log: L,
        credits: Option<Arc<Credits<L>>>,
    ) -> (Tested<L>, Seen) {
        let provider = self
            .model
            .unwrap_or_else(|| ScriptedModel::new(self.answers));
        let provider = match self.gate {
            Some(gate) => provider.gated(gate),
            None => provider,
        };
        let seen = provider.seen();
        let computers = LocalComputers::for_here(environment(&self.variables))
            .with_scratch(dir.with_extension("scratch"));
        let computers = match &self.worktrees {
            Some(base) => computers.with_worktrees(base),
            None => computers,
        };
        let computers = match &self.project_base {
            Some(base) => computers.with_projects(base),
            None => computers,
        };
        // One computer for every test server, as this machine's own id is kept across runs.
        let computers = computers.known_as(ComputerId::from_uuid(uuid::Uuid::from_u128(7)));
        let (computers, info) = computers.open_here(dir).await.unwrap();
        let home = self.home;
        keep_folder(&log, home, &info).await;
        let workspace = Workspace::new(computers, info);
        let workspace = match self.credentials {
            Some(credentials) => workspace.with_credentials(credentials),
            None => workspace,
        };
        let projects = self
            .projects
            .unwrap_or_else(|| Arc::new(InMemoryProjects::new()) as Arc<dyn Projects>);
        let wiring = Wiring {
            clock: Box::new(self.clock),
            entropy: Box::new(CountingEntropy::starting_at(self.seed << 32)),
            tasks: self.limits,
            memory: self.memory,
            models: self.models,
            providers: self.providers,
            git_hosts: self.hosts,
            connections: self.connections,
            project_folders: self.project_folders,
            trust: self.trust,
            browsers: self.browsers,
            accounts: self.accounts,
            mail: self.mail,
            origin: self.origin,
            signup: self.signup,
            forwarded: self.forwarded,
            leases: self.leases,
            tools: self.tools,
            awake: self.awake,
            worktrees: self.release,
            workers: self.workers,
            identity_provider: self.identity_provider,
            homes: self.homes,
            credits,
            ..Wiring::new(provider, log, InMemoryBlobs::new(), workspace, projects)
        };
        let server = Server::new(wiring, self.settings, home.org(), self.answering);
        keep_own_project(&server, home).await;
        (server, seen)
    }
}

/// Keeps the folder `info` opened as the server's own project's, as glave keeps the folder it
/// runs in: at a fixed time with fixed ids, for the same reason as the project itself. A second
/// server on a shared log finds it there already.
async fn keep_folder<L: EventLog>(log: &L, home: Scope, info: &WorkspaceInfo) {
    let at = At::try_from("2026-10-01T00:00:00.000Z".to_owned()).unwrap();
    let header = Header {
        id: EventId::new(at.timestamp(), [9; 10]).unwrap(),
        thread: glave_ports::stream::Reserved::Project.id(),
        seq: Seq::FIRST,
        at,
    };
    let set = ProjectFolderSet {
        folder: uuid::Uuid::from_u128(3),
        source: FolderSource::Local,
        checkout: Checkout {
            computer: info.computer.as_uuid(),
            path: info.root.clone(),
        },
        by: None,
    };
    let _ = log.append(&home, &[encode(header, &set).unwrap()]).await;
}

/// This process's variables, with `given` in place of its own of the same names.
fn environment(given: &[(String, PathBuf)]) -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| !given.iter().any(|(set, _)| name == set.as_str()))
        .chain(
            given
                .iter()
                .map(|(name, value)| (name.into(), value.into())),
        )
        .collect()
}

/// The server of `Setup::asking`.
pub(crate) async fn server(dir: &Path, answers: Vec<Completion>, answering: Answering) -> Tested {
    Setup::asking(answers, answering).build(dir).await.0
}

/// The server's own project, kept as its person's project at a fixed time without reading the
/// clock, so nothing a test checks depends on when it was first asked for. A second server on
/// another's projects finds it there already.
async fn keep_own_project<L: EventLog + 'static>(server: &Tested<L>, home: Scope) {
    let at = At::try_from("2026-10-01T00:00:00.000Z".to_owned()).unwrap();
    let project = Project {
        id: home.project(),
        kind: ProjectKind::Project,
        name: "test".into(),
        owner: Principal::person(server.person().unwrap()),
        created_at: at,
        updated_at: at,
    };
    let projects = server.projects();
    if projects
        .get(home.org(), project.id)
        .await
        .unwrap()
        .is_none()
    {
        projects.create(home.org(), &project).await.unwrap();
    }
}

/// Leases in memory that two servers can share, as two processes share a database, on a clock
/// the test moves.
pub(crate) fn shared_leases() -> (ManualClock, Arc<InMemoryLeases>) {
    let clock = ManualClock::at_millis(1_791_001_000_000);
    let leases = Arc::new(InMemoryLeases::new(Arc::new(clock.clone())));
    (clock, leases)
}

/// A log in memory that two servers can share, as two processes share a database.
#[derive(Clone, Default)]
pub(crate) struct Shared(pub Arc<InMemoryLog>);

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
        org: OrgId,
        thread: ThreadId,
    ) -> impl Future<Output = Result<Option<Located>, LogError>> + Send {
        self.0.locate(org, thread)
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

/// Settings where nothing asks but the risky actions: Full access, since a test server's
/// commands run without a sandbox.
fn full_access() -> Settings {
    let mut settings = Settings::new("test-model", "script");
    settings.permissions = Permissions::FullAccess;
    settings
}
