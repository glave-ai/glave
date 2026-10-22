//! Wiring glave together: the session server on this computer's store and workspace, with
//! the folder glave runs in as its project's, its models, MCP servers, memory and projects.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use glave_computer::WorkerComputers;
use glave_config::policy::{self, Choices};
use glave_config::trust::Asking;
use glave_harness::{AGENTS_MD, Settings, Workspace, agents_md};
use glave_memory::Tidier;
use glave_ports::computer::GitCredentials;
use glave_ports::settings::Hooks;
use glave_ports::{ProjectId, SystemClock};
use glave_process::Processes;
use glave_session::{Answering, GitHosts, ProjectTrust, Providers, Rereads, Server, Wiring};

use crate::args::Args;
use crate::catalogs::Catalogs;
use crate::client::Glave;
use crate::config::{self, Endpoint};
use crate::credentials::Hosts;
use crate::data::Data;
use crate::gateway::GatewaySwitch;
use crate::gateway::remote::Remote;
use crate::mcp::Servers;
use crate::project_folders::ProjectFolders;
use crate::providers::{Built, Lineup, ModelAccounts};
use crate::reread::Rereading;
use crate::setup::{self, Setup};
use crate::trust::Trust;
use crate::{home, identity};

/// The server for this folder, and what the terminal needs to show it.
pub(crate) struct Wired {
    pub server: Glave,
    pub(crate) servers: Arc<Servers>,
    /// The default model's endpoint; none while no model is set up.
    pub(crate) endpoint: Option<Endpoint>,
    /// The project of the folder glave runs in, where the terminal's threads are.
    pub(crate) project: ProjectId,
    pub(crate) root: PathBuf,
    /// The settings in use, for the banner and receipts.
    pub(crate) choices: Choices,
    /// The workers threads work on, with `--on cloud`.
    pub(crate) workers: Option<Arc<WorkerComputers>>,
    /// Memory, when it is on.
    pub(crate) memory: Option<crate::memory::Running>,
    /// The store's log, for the daily work of the person's server.
    pub(crate) log: crate::data::DataLog,
    /// The store's blobs and its organization, which the daily work sweeps.
    pub(crate) blobs: crate::data::DataBlobs,
    pub(crate) org: glave_ports::OrgId,
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
/// folder: their own, after the catalog of the gateway elsewhere.
async fn mcp_servers(
    setup: &Setup,
    remote: Option<&Remote>,
    processes: &Processes,
) -> anyhow::Result<Arc<Servers>> {
    let chosen = setup.choices.mcp.as_ref();
    let constraints = &setup.loaded.constraints;
    let home = home::folder()?;
    let wanted = crate::mcp::wanted(&home, chosen, constraints, remote.is_some()).await?;
    let root = home::person()?;
    Ok(Arc::new(Servers::start(wanted, &root, remote, processes)))
}

/// What a thread opens with where `root` is: the person's own AGENTS.md, then the folder's.
async fn opening(root: &Path) -> anyhow::Result<Option<String>> {
    let personal = home::folder()?.join(AGENTS_MD);
    let instructions = agents_md(root, Some(&personal)).await?;
    Ok(instructions.map(|loaded| loaded.text))
}

/// Builds the session server on this computer's store and the current folder.
pub(crate) async fn wire(
    args: &Args,
    answering: Answering,
    asking: Asking,
) -> anyhow::Result<Wired> {
    let folder = std::env::current_dir()?;
    let mut setup = Setup::load(args, &folder).await?;
    let telemetry = crate::telemetry::start_for(args.log.as_deref(), &setup.choices)?;
    let database = home::database().await?;
    let processes = home::reaped().await?;
    let workspace =
        crate::sandboxing::workspace(&folder, &mut setup, &database, &processes).await?;
    let choices = &setup.choices;
    let (remote, configured) = crate::providers::endpoint(choices)?;
    let store = store(&setup).await?;
    let identity = identity::load_or_make(&store).await?;
    let (asked, told) = asked_here(&folder, &setup, &store, identity.person, asking).await?;
    let lineup = Lineup::new(&setup, configured);
    let lined = line_up(args, lineup, &store, identity.person, remote.is_some()).await?;
    let (workspace, hosts) = with_hosts(args, &store, identity.person, choices, workspace).await?;
    let opening = opening(workspace.root()).await?;
    let mut settings = settings_for(args, choices, &lined, identity, opening)?;
    let root = workspace.root().to_owned();
    settings.commands = crate::commands::find(&root).await?;
    let servers = mcp_servers(&setup, remote.as_ref(), &processes).await?;
    let scope = scope_here(args, choices, &store, &folder, &root, identity.person).await?;
    let (on_workers, workers) =
        crate::cloud::workers(choices, &setup.loaded.constraints, &folder, scope.project()).await?;
    let memory = crate::memory::start_for(&store, &setup, identity.person, scope).await?;
    let projects = projects_for(&setup, &servers, &lined.switch, memory.as_ref(), asked)?;
    let set_aside = projects.set_aside(&folder).await;
    (told.iter().chain(&set_aside)).for_each(|line| eprintln!("glave: {line}"));
    tidy(memory.as_ref(), &lined.switch, settings.model.clone());
    let rereads = rereading(args, &folder, choices, &lined, &servers, &projects).await?;
    let wiring = Wiring {
        models: lined.built.models.offered,
        modes: policy::thread_modes(choices, &setup.loaded.constraints),
        providers: lined.accounts.map(|kept| kept as Arc<dyn Providers>),
        rereads: Some(rereads),
        git_hosts: hosts.map(|hosts| hosts as Arc<dyn GitHosts>),
        web: crate::project_folders::web_offered(&choices.web.value)?,
        tools: crate::skills::offered(&root).await?,
        // The person's own servers, for their threads alone.
        own: servers.sources(),
        worktree: choices.worktree.value,
        hooks: Hooks::new(setup.hooks.clone())?,
        workers: on_workers,
        tasks: task_limits(&setup.loaded.constraints),
        memory: memory.as_ref().map(crate::memory::Running::handle),
        project_folders: Some(Arc::clone(&projects) as Arc<dyn glave_harness::ProjectFolders>),
        trust: Some(Arc::clone(&projects) as Arc<dyn ProjectTrust>),
        browsers: crate::serving::browsers(args, choices, &store, identity.person).await?,
        origin: choices.origin.as_ref().map(|origin| origin.value.clone()),
        leases: Some(store.leases()),
        awake: crate::awake::for_server(args, choices, &processes, &store, identity.person).await?,
        ..Wiring::new(
            lined.switch,
            store.log(),
            store.blobs(),
            workspace,
            store.projects(),
        )
    };
    let server = Server::new(wiring, settings, scope.org(), answering);
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
        workers,
        memory,
        log: store.log(),
        blobs: store.blobs(),
        org: scope.org(),
        projects,
        reach,
        _telemetry: telemetry,
    })
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
    start: &Choices,
    lined: &Lined,
    servers: &Arc<Servers>,
    projects: &Arc<ProjectFolders>,
) -> anyhow::Result<Arc<dyn Rereads>> {
    let rereading = Rereading::new(crate::reread::Parts {
        args: args.clone(),
        folder: folder.to_owned(),
        home: home::folder()?,
        start: start.clone(),
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
) -> anyhow::Result<Arc<ProjectFolders>> {
    let own = crate::project_folders::Own {
        loaded: setup.loaded.clone(),
        choices: setup.choices.clone(),
        taken: servers.names(),
        models: models.clone(),
        memory: memory.map(|memory| Arc::clone(&memory.memory)),
        clock: Box::new(SystemClock),
        processes: servers.processes().clone(),
        trust: asked.trust,
    };
    let projects = Arc::new(ProjectFolders::new(home::folder()?, own, asked.later));
    ProjectFolders::tidy(&projects);
    Ok(projects)
}

/// How the harness runs: the default model, the instructions a thread opens with when it has
/// no folder of its own here (a worker's, from `opening`), and the choices.
fn settings_for(
    args: &Args,
    choices: &Choices,
    lined: &Lined,
    identity: identity::Identity,
    opening: Option<String>,
) -> anyhow::Result<Settings> {
    let (model, named) = lined.built.models.default.clone().unwrap_or_default();
    let mut settings = Settings::new(model, named);
    settings.instructions = opening;
    settings.approval = choices.approval.value;
    settings.max_steps = args.max_steps;
    settings.subagents = true; // As Claude Code, Codex and OpenCode offer it.
    // As Claude.ai, ChatGPT and OpenCode title chats; a one-shot prompt turns them off (its
    // flags, `setup.rs`).
    settings.titles = choices.titles.value;
    settings.judge = choices.judge.value.clone();
    (settings.person, settings.agent) = (Some(identity.person), Some(identity.agent));
    let window = config::context_window()?.unwrap_or(settings.compaction.window_tokens);
    settings.compaction.window_tokens = window;
    Ok(settings)
}

/// The git hosts `person` connected, when they may connect them here, and `workspace` with
/// every thread's commands asking them for credentials at each use.
async fn with_hosts(
    args: &Args,
    store: &Data,
    person: glave_event::PrincipalId,
    choices: &Choices,
    workspace: Workspace,
) -> anyhow::Result<(Workspace, Option<Arc<Hosts>>)> {
    let others_sign_in = !args.issuers.is_empty();
    let github = choices.github.as_ref().map(|github| &github.value);
    let hosts = Hosts::for_person(store, person, github, others_sign_in).await?;
    Ok(match hosts {
        Some(hosts) => {
            let asked = Arc::clone(&hosts) as Arc<dyn GitCredentials>;
            (workspace.with_credentials(asked), Some(hosts))
        }
        None => (workspace, None),
    })
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

/// Lines up the models of `lineup` and of the model accounts `person` connected, which they
/// may connect here unless an organization sets up the models: through its gateway
/// (`through_gateway`), or on a server people sign in to with OIDC.
async fn line_up(
    args: &Args,
    lineup: Lineup,
    store: &Data,
    person: glave_event::PrincipalId,
    through_gateway: bool,
) -> anyhow::Result<Lined> {
    let ledgers = if through_gateway || !args.issuers.is_empty() {
        None
    } else {
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
    let switch = GatewaySwitch::new(built.gateway.take());
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
/// daily check.
async fn store(setup: &Setup) -> anyhow::Result<Data> {
    let store = Data::chosen(&setup.choices).await?;
    crate::worktrees::check_daily(&store.log()).await;
    Ok(store)
}

/// A server in this process for a command's requests, where none runs for the person and no
/// one is at the terminal: answered by no one, asking nothing.
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
