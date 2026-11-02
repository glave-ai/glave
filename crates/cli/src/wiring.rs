//! Wiring glave together: the session server on this computer's store and workspace, with
//! the folder glave runs in as its project's, its models, MCP servers, memory and projects.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use glave_computer::WorkerComputers;
use glave_config::policy::{self, Choices};
use glave_config::trust::Asking;
use glave_gateway::Credits;
use glave_harness::{AGENTS_MD, Own, Settings, Workspace, agents_md};
use glave_memory::Tidier;
use glave_ports::computer::{GitCredentials, Sandbox};
use glave_ports::secrets::KnownSecrets;
use glave_ports::settings::Hooks;
use glave_ports::{OsEntropy, ProjectId, SystemClock};
use glave_process::Processes;
use glave_session::{
    Answering, Connections, GitHosts, ProjectTrust, Providers, Rereads, Secrets, Server, Wiring,
};

use crate::args::Args;
use crate::catalogs::Catalogs;
use crate::client::Glave;
use crate::config::{self, Endpoint};
use crate::connected::{Connected, WebSearch};
use crate::credentials::Hosts;
use crate::data::{Data, DataLog};
use crate::gateway::GatewaySwitch;
use crate::gateway::remote::Remote;
use crate::mcp::Servers;
use crate::project_folders::ProjectFolders;
use crate::providers::{Built, Lineup, ModelAccounts};
use crate::reread::Rereading;
use crate::setup::{self, Setup};
use crate::trust::Trust;
use crate::worktrees::OnThisMachine;
use crate::{home, identity};

/// The variable that puts a stand-in in place of Exa's server for web search, for tests, which
/// must never reach Exa: every glave a test starts is given one.
const SEARCH_STAND_IN: &str = "GLAVE_SEARCH_URL";

/// The server for this folder, and what the terminal needs to show it.
pub(crate) struct Wired {
    pub server: Glave,
    pub(crate) servers: Arc<Servers>,
    /// The default model's endpoint; none while no model is set up.
    pub(crate) endpoint: Option<Endpoint>,
    /// The project of the folder glave runs in, where the terminal's threads are: the person's
    /// Chats where that folder is no project of its own ([`start_folder`]).
    pub(crate) project: ProjectId,
    pub(crate) root: PathBuf,
    /// The settings in use, for the banner and receipts.
    pub(crate) choices: Choices,
    /// The most a sandbox here allows, which each mode narrows: for the banner and receipts.
    pub(crate) sandbox: Option<Sandbox>,
    /// The workers threads work on, with `--on cloud`.
    pub(crate) workers: Option<Arc<WorkerComputers>>,
    /// Memory, when it is on.
    pub(crate) memory: Option<crate::memory::Running>,
    /// The store's log, for the daily work of the person's server.
    pub(crate) log: DataLog,
    /// The store, whose blobs the daily work sweeps.
    pub(crate) store: Data,
    /// Each project's own servers, hooks and settings, and what it asks to be trusted.
    pub(crate) projects: Arc<ProjectFolders>,
    /// How people reach the server from elsewhere, and its HTTPS of its own.
    pub(crate) reach: crate::serving::Reach,
    /// Logs and telemetry, stopped last.
    pub(crate) _telemetry: crate::telemetry::Telemetry,
}

/// The task limits every `[tasks]` rule of `constraints` sets at once.
fn task_limits(constraints: &policy::Constraints) -> glave_session::TaskLimits {
    let rules = constraints.task_limits();
    glave_session::TaskLimits {
        max_running: rules.max_running,
        max_schedules: rules.max_schedules,
    }
}

/// The MCP servers the setup turns on for the person's threads everywhere, in their home
/// folder: their own, after the catalog of the gateway elsewhere, remote ones signed in with
/// what the person `connected`.
async fn mcp_servers(
    setup: &Setup,
    remote: Option<&Remote>,
    processes: &Processes,
    connected: Option<&Arc<Connected>>,
    reachable: glave_mcp::Reachable,
) -> anyhow::Result<Arc<Servers>> {
    let chosen = setup.choices.mcp.as_ref();
    let constraints = &setup.loaded.constraints;
    let home = home::folder()?;
    let signed_in = match connected {
        Some(connected) => connected.addresses().await,
        None => BTreeSet::new(),
    };
    let through_gateway = remote.is_some();
    let wanted =
        crate::mcp::wanted(&home, chosen, constraints, through_gateway, &signed_in).await?;
    let root = home::person()?;
    let connected = connected.map(Arc::clone);
    Ok(Arc::new(Servers::start(
        wanted, &root, remote, processes, connected, reachable,
    )))
}

/// What a thread opens with where `root` is: the person's own AGENTS.md, then the folder's.
async fn opening(root: &Path) -> anyhow::Result<Option<String>> {
    let personal = home::folder()?.join(AGENTS_MD);
    let instructions = agents_md(root, Some(&personal)).await?;
    Ok(instructions.map(|loaded| loaded.text))
}

/// Builds the session server on this computer's store and the current folder.
#[expect(
    clippy::too_many_lines,
    reason = "one list of every part a server is wired from, each a line"
)]
pub(crate) async fn wire(
    args: &Args,
    answering: Answering,
    asking: Asking,
) -> anyhow::Result<Wired> {
    let folder = std::env::current_dir()?;
    let mut setup = Setup::load(args, &folder).await?;
    let telemetry = crate::telemetry::start_for(args.log.as_deref(), &setup.choices)?;
    let processes = home::reaped().await?;
    let workspace = crate::sandboxing::workspace(&folder, &mut setup, &processes).await?;
    let choices = &setup.choices;
    let (remote, configured) = crate::providers::endpoint(choices)?;
    let store = store(&setup, &processes).await?;
    let identity = identity::load_or_make(&store).await?;
    let (asked, told) = asked_here(&folder, &setup, &store, identity.person, asking).await?;
    let lineup = Lineup::new(&setup, configured);
    let credits = credits(choices, &store);
    let floor = choices.router_floor.as_ref().map(|floor| &floor.value);
    let through_gateway = remote.is_some();
    let lined = line_up(
        args,
        lineup,
        &store,
        identity.person,
        (credits.as_ref(), floor),
        through_gateway,
    );
    let lined = lined.await?;
    let (workspace, hosts, connected) =
        signed(args, &store, identity.person, &setup, workspace).await?;
    let (workspace, secrets) = kept_secrets(args, &store, identity.person, workspace).await?;
    let settings = settings_for(args, &setup, &lined, identity, workspace.root()).await?;
    let root = workspace.root().to_owned();
    let servers = mcp_servers(
        &setup,
        remote.as_ref(),
        &processes,
        connected.as_ref(),
        crate::mcp::own_reachable(args, choices),
    )
    .await?;
    let scope = scope_here(args, choices, &store, &folder, &root, identity.person).await?;
    let constraints = &setup.loaded.constraints;
    let (on_workers, workers, homes) = crate::cloud::workers(
        choices,
        constraints,
        &folder,
        scope.project(),
        &processes,
        store.leases(),
    )
    .await?;
    let memory = crate::memory::start_for(&store, &setup, identity.person, scope).await?;
    let reachable = crate::mcp::reachable(args, &setup.choices);
    let projects = projects_for(
        &setup,
        &servers,
        &lined.switch,
        memory.as_ref(),
        asked,
        reachable,
    )?;
    say(&told, &projects, &folder).await;
    // Memory is tidied once the server answers the filters its calls keep to.
    let (filters, tidier) = (lined.switch.filters(), lined.switch.clone());
    let tidy_model = settings.model.clone();
    let wiring = Wiring {
        // Before the lineup's parts move into the rest.
        rereads: Some(rereading(args, &folder, &setup, &lined, &servers, &projects).await?),
        models: lined.built.models.offered,
        providers: lined.accounts.map(|kept| kept as Arc<dyn Providers>),
        git_hosts: hosts.map(|hosts| hosts as Arc<dyn GitHosts>),
        connections: connected.map(|connected| connected as Arc<dyn Connections>),
        secrets,
        web: crate::project_folders::web_offered(&choices.web.value)?,
        skills: crate::skills::offered(&root).await?,
        // The person's own servers, for their threads alone.
        own: servers.sources(),
        worktree: choices.worktree.value,
        hooks: Hooks::new(setup.hooks.clone())?,
        workers: on_workers,
        identity_provider: !args.issuers.is_empty(),
        tasks: task_limits(constraints),
        memory: memory.as_ref().map(crate::memory::Running::handle),
        project_folders: Some(Arc::clone(&projects) as Arc<dyn glave_harness::ProjectFolders>),
        trust: Some(Arc::clone(&projects) as Arc<dyn ProjectTrust>),
        browsers: crate::serving::browsers(args, choices, &store, identity.person).await?,
        accounts: crate::serving::accounts(args, choices, &store).await?,
        mail: crate::serving::mail(choices)?,
        origin: choices.origin.as_ref().map(|origin| origin.value.clone()),
        signup: choices.signup.as_ref().is_some_and(|signup| signup.value),
        forwarded: choices.forwarded.as_ref().map(|forwarded| forwarded.value),
        leases: Some(store.leases()),
        awake: crate::awake::for_server(args, choices, &processes),
        worktrees: Some(Arc::new(OnThisMachine(store.log(), processes.clone()))),
        homes,
        credits,
        router_floor: crate::gateway::router_floor(floor),
        ..Wiring::new(
            lined.switch,
            store.log(),
            store.blobs(),
            workspace,
            store.projects(),
        )
    };
    let server = Server::new(wiring, settings, scope.org(), answering);
    server.answer_filters(&filters);
    tidy(memory.as_ref(), &tidier, tidy_model);
    let reach = crate::serving::reach(args, choices, &store, identity.person).await?;
    if start_folder(args, &setup.choices, &folder) {
        keep_here(&server, scope.project(), &root).await?;
    }
    Ok(Wired {
        server,
        servers,
        endpoint: lined.built.endpoint,
        project: scope.project(),
        root,
        choices: setup.choices,
        sandbox: setup.sandbox,
        workers,
        memory,
        log: store.log(),
        store: store.clone(),
        projects,
        reach,
        _telemetry: telemetry,
    })
}

/// Says on standard error what glave `told` the person about `folder`, and the settings of
/// `projects` it set aside there.
async fn say(told: &[String], projects: &ProjectFolders, folder: &Path) {
    let set_aside = projects.set_aside(folder).await;
    (told.iter().chain(&set_aside)).for_each(|line| eprintln!("glave: {line}"));
}

/// The scope glave's own work here keeps to: the project of `root`, the workspace of `folder`,
/// the folder it starts in, when that is a project of its own ([`start_folder`]); else
/// `person`'s Chats.
async fn scope_here(
    args: &Args,
    choices: &Choices,
    store: &Data,
    folder: &Path,
    root: &Path,
    person: glave_event::PrincipalId,
) -> anyhow::Result<glave_ports::Scope> {
    if start_folder(args, choices, folder) {
        crate::projects::here(store, root, person).await
    } else {
        crate::projects::personal(store, person).await
    }
}

/// Whether `folder`, the folder glave starts in, is a project of its own: for a turn run in
/// it, or a program's own server; for the person's server, which has no folder of its own
/// (`specs/one-server.md`), only where its threads go to workers, which clone that folder's
/// repository; never for a server that only answers a command's requests ([`here`]), and
/// never where the folder is never a project.
fn start_folder(args: &Args, choices: &Choices, folder: &Path) -> bool {
    let own = if args.person_s_server() {
        choices.on.value == policy::On::Cloud
    } else {
        args.stdio || args.turn()
    };
    own && !crate::projects::never_a_project(folder)
}

/// Keeps `root`, the folder glave runs in, as the folder of its project, `project`: where its
/// threads work.
async fn keep_here(server: &Glave, project: ProjectId, root: &Path) -> anyhow::Result<()> {
    server
        .keep_folder(project, &root.to_string_lossy())
        .await
        .with_context(|| format!("could not keep {} as its project's folder", root.display()))
}

/// What reads the person's settings again for a server started in `folder` with `args` and
/// the choices `start`, which its models, servers and projects follow.
async fn rereading(
    args: &Args,
    folder: &Path,
    start: &Setup,
    lined: &Lined,
    servers: &Arc<Servers>,
    projects: &Arc<ProjectFolders>,
) -> anyhow::Result<Arc<dyn Rereads>> {
    let rereading = Rereading::new(crate::reread::Parts {
        args: args.clone(),
        folder: folder.to_owned(),
        home: home::folder()?,
        start: start.choices.clone(),
        here: start.sandbox.as_ref().map(|sandbox| sandbox.level),
        accounts: lined.accounts.clone(),
        switch: lined.switch.clone(),
        servers: Arc::clone(servers),
        projects: Arc::clone(projects),
    });
    Ok(Arc::new(rereading.await))
}

/// What the person decided about each project, and whether they answer what a project asks
/// later, from an app.
struct Asked {
    trust: Trust,
    later: bool,
}

/// What the person decided about projects, in `store`, and what to tell them of the project
/// around `folder`: the terminal asks about it before glave starts, as `asking` lets; an app
/// asks later.
async fn asked_here(
    folder: &Path,
    setup: &Setup,
    store: &Data,
    person: glave_event::PrincipalId,
    asking: Asking,
) -> anyhow::Result<(Asked, Vec<String>)> {
    let trust = Trust::open(store, person).await?;
    let told = match asking {
        Asking::Later => Vec::new(),
        asking => setup::ask_here(folder, setup, asking, &trust).await?,
    };
    let later = asking == Asking::Later;
    Ok((Asked { trust, later }, told))
}

/// Each project as this glave reads it, beside the `servers` it runs for the person's threads
/// everywhere, offering the models of `models`, with `memory`, under what the person `asked`.
fn projects_for(
    setup: &Setup,
    servers: &Servers,
    models: &GatewaySwitch,
    memory: Option<&crate::memory::Running>,
    asked: Asked,
    reachable: glave_mcp::Reachable,
) -> anyhow::Result<Arc<ProjectFolders>> {
    let own = crate::project_folders::Own {
        loaded: setup.loaded.clone(),
        choices: setup.choices.clone(),
        taken: servers.names(),
        models: models.clone(),
        memory: memory.map(|memory| Arc::clone(&memory.memory)),
        clock: Box::new(SystemClock),
        processes: servers.processes().clone(),
        reachable,
        trust: asked.trust,
    };
    let projects = Arc::new(ProjectFolders::new(home::folder()?, own, asked.later));
    ProjectFolders::tidy(&projects);
    Ok(projects)
}

/// How the harness runs: the default model, the instructions a thread opens with when it has
/// no folder of its own here (a worker's, what `root` opens with), the custom commands
/// `root` holds, and the choices.
async fn settings_for(
    args: &Args,
    setup: &Setup,
    lined: &Lined,
    identity: identity::Identity,
    root: &Path,
) -> anyhow::Result<Settings> {
    let choices = &setup.choices;
    let (model, named) = lined.built.models.default.clone().unwrap_or_default();
    let mut settings = Settings::new(model, named);
    settings.instructions = opening(root).await?;
    settings.commands = crate::commands::find(root).await?;
    settings.permissions = choices.permissions.value;
    settings.highest = setup.loaded.constraints.highest_permissions().0;
    settings.max_steps = args.max_steps;
    settings.subagents = true; // As Claude Code, Codex and OpenCode offer it.
    settings.plans = true; // As Codex's update_plan and Claude Code's todo list are offered.
    // As Claude.ai, ChatGPT and OpenCode title chats; a one-shot prompt turns them off (its
    // flags, `setup.rs`).
    settings.titles = choices.titles.value;
    settings.judge = choices.judge.value.clone();
    (settings.person, settings.agent) = (Some(identity.person), Some(identity.agent));
    // What `install` puts in the person's own folder: not where people sign in with an
    // identity provider, whose own skills and servers this server doesn't keep.
    if args.issuers.is_empty() {
        settings.own = Some(Own {
            folder: home::folder()?,
            servers: setup.loaded.constraints.own_mcp_forbidden().is_none(),
        });
    }
    let window = config::context_window()?.unwrap_or(settings.compaction.window_tokens);
    settings.compaction.window_tokens = window;
    Ok(settings)
}

/// The git hosts `person` connected, when they may connect them here, and `workspace` with
/// every thread's commands asking them for credentials at each use; and the remote servers they
/// sign in to, likewise, with web search where they may add servers of their own.
async fn signed(
    args: &Args,
    store: &Data,
    person: glave_event::PrincipalId,
    setup: &Setup,
    workspace: Workspace,
) -> anyhow::Result<(Workspace, Option<Arc<Hosts>>, Option<Arc<Connected>>)> {
    let others_sign_in = !args.issuers.is_empty();
    let github = setup.choices.github.as_ref().map(|github| &github.value);
    let hosts = Hosts::for_person(store, person, github, others_sign_in).await?;
    // Web search is the person's to turn on where they may add servers of their own.
    let forbidden = setup.loaded.constraints.own_mcp_forbidden().is_some();
    let search = (!forbidden).then(|| WebSearch::at(std::env::var(SEARCH_STAND_IN).ok()));
    let home = home::folder()?;
    let reachable = crate::mcp::reachable(args, &setup.choices);
    let connected =
        Connected::for_person(store, person, &home, others_sign_in, search, reachable).await?;
    Ok(match hosts {
        Some(hosts) => {
            let asked = Arc::clone(&hosts) as Arc<dyn GitCredentials>;
            (workspace.with_credentials(asked), Some(hosts), connected)
        }
        None => (workspace, None, connected),
    })
}

/// The secrets `person` adds, when they may add them here: everywhere but a server people sign
/// in to with OIDC, as for git hosts; and `workspace` with every command seeing their
/// placeholders and every tool's output masking their values.
async fn kept_secrets(
    args: &Args,
    store: &Data,
    person: glave_event::PrincipalId,
    workspace: Workspace,
) -> anyhow::Result<(Workspace, Option<Arc<Secrets<DataLog>>>)> {
    if !args.issuers.is_empty() {
        return Ok((workspace, None));
    }
    let known = KnownSecrets::default();
    let ledger = crate::accounts::open(store, person).await?;
    let secrets = Secrets::open(ledger, known.clone()).await?;
    Ok((workspace.hiding(known), Some(Arc::new(secrets))))
}

/// Tidies memory in the background, when it is on: tidy-up asks the gateway too, which sends
/// it on the `summary` route.
fn tidy(memory: Option<&crate::memory::Running>, switch: &GatewaySwitch, model: String) {
    if let Some(memory) = memory {
        memory.tidy_with(Tidier {
            provider: switch.clone(),
            model,
        });
    }
}

/// The models a run offers and the gateway to them.
struct Lined {
    built: Built,
    switch: GatewaySwitch,
    /// The person's model accounts, when they may connect them here.
    accounts: Option<Arc<ModelAccounts>>,
}

/// The organizations' credits in `store`, where the server's own settings keep them
/// (`[credits]`), each organization limited to the monthly limit they name until its Owner
/// sets another.
fn credits(choices: &Choices, store: &Data) -> Option<Arc<Credits<DataLog>>> {
    let chosen = choices.credits.as_ref()?;
    Some(Arc::new(Credits::new(
        store.log(),
        Box::new(SystemClock),
        Box::new(OsEntropy),
        chosen.value.monthly_limit_usd,
    )))
}

/// Lines up the models of `lineup` and of the model accounts `person` connected, which they
/// may connect here unless an organization sets up the models: through its gateway
/// (`through_gateway`), or on a server people sign in to with OIDC, unless it is the server's
/// own OpenRouter, whose catalog is kept the same way. Their calls are charged to `credits`,
/// where the server keeps them.
async fn line_up(
    args: &Args,
    lineup: Lineup,
    store: &Data,
    person: glave_event::PrincipalId,
    (credits, floor): (
        Option<&Arc<Credits<DataLog>>>,
        Option<&policy::RouterFloorFile>,
    ),
    through_gateway: bool,
) -> anyhow::Result<Lined> {
    let no_ledgers = !args.issuers.is_empty() && lineup.own_account().is_none();
    let ledgers = if through_gateway || no_ledgers {
        None
    } else {
        crate::providers::check_stand_in()?;
        let accounts = crate::accounts::open(store, person).await?;
        Some((accounts, Catalogs::open(store, person).await?))
    };
    let (accounts, catalogs) = match &ledgers {
        Some((accounts, catalogs)) => (
            accounts.accounts().await?.providers,
            catalogs.latest().await?,
        ),
        None => (Vec::new(), Vec::new()),
    };
    // An account that no longer lines up with the settings never stops glave starting.
    let mut built = match lineup.build(&accounts, &catalogs) {
        Ok(built) => built,
        Err(error) if !accounts.is_empty() => {
            eprintln!("glave: the model accounts you connected are set aside: {error:#}");
            lineup.build(&[], &[])?
        }
        Err(error) => return Err(error),
    };
    let floor = crate::gateway::router_floor(floor);
    let switch = GatewaySwitch::new(built.gateway.take(), credits.cloned(), floor);
    let model_accounts = ledgers.map(|(accounts, catalogs)| {
        Arc::new(ModelAccounts::new(
            accounts,
            catalogs,
            lineup,
            switch.clone(),
        ))
    });
    Ok(Lined {
        built,
        switch,
        accounts: model_accounts,
    })
}

/// The data store the setup's choices name, once this machine's worktrees have had their
/// daily check, git run as one of `processes`.
async fn store(setup: &Setup, processes: &Processes) -> anyhow::Result<Data> {
    let store = Data::chosen(&setup.choices).await?;
    crate::worktrees::check_daily(&store.log(), processes).await;
    Ok(store)
}

/// A server in this process for a command's requests, where none runs for the person and no
/// one is at the terminal: answered by no one, asking nothing. Asked for no turn, it makes no
/// project of the folder it starts in ([`start_folder`]), as the person's server makes none.
pub(crate) async fn here() -> anyhow::Result<Wired> {
    wire(&Args::default(), Answering::NoOne, Asking::NoOne).await
}

impl Wired {
    /// Lets go of what the server started: workers, MCP servers, projects' tools, memory.
    pub(crate) async fn close(&self) {
        if let Some(workers) = &self.workers {
            crate::cloud::release(&self.server, workers).await;
        }
        self.servers.close().await;
        self.projects.close().await;
        if let Some(memory) = &self.memory {
            // The session's forgets batched into a project-key rotation.
            memory.housekeep().await;
            memory.memory.close().await;
        }
    }
}
