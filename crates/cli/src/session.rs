//! Wiring glave together: the session server on this computer's store and workspace, with
//! the terminal as its client, for one turn or an interactive session.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context as _;
use glave_computer::{LocalComputers, WorkerComputers};
use glave_event::ThreadId;
use glave_harness::{
    AGENTS_MD, ApprovalMode, Cancel, DEFAULT_BUDGET, Goal, Hooks, OsEntropy, Projects as _,
    Protections, Sandbox, Settings, SystemClock, Workspace, agents_md,
};
use glave_memory::Tidier;
use glave_ports::{Credentials, SpaceId, ToolSource};
use glave_process::Processes;
use glave_session::protocol::{Reply, Request};
use glave_session::{Answering, GitHosts, ProjectTrust, Providers, Server, Wiring};

use crate::accounts::Ledger;
use crate::args::{self, Action, Args, USAGE};
use crate::client::{Client, Glave, Session};
use crate::config::{self, Endpoint};
use crate::credentials::Hosts;
use crate::data::Data;
use crate::gateway::Switch;
use crate::gateway::remote::Remote;
use crate::input::{CtrlC, Input};
use crate::mcp::Servers;
use crate::projects::Projects;
use crate::providers::{Built, Host, Lineup};
use crate::setup::{self, Setup};
use crate::terminal::Printer;
use crate::{home, identity};
use glave_config::policy::{self, Choices, On, Source};
use glave_config::trust::Asking;

/// A command's own entry: it reads the words after its name.
type Subcommand = fn(&[String]) -> ExitCode;

/// The commands that are their first word, each reading the words after it. Only as the
/// first word: a prompt starting with one goes after `--`.
const SUBCOMMANDS: [(&str, Subcommand); 10] = [
    ("server", crate::server::main),
    ("computer", crate::computer::main),
    ("worktrees", crate::worktrees::main),
    ("threads", crate::threads::main),
    ("memory", crate::memory::main),
    ("connect", crate::connections::connect_main),
    ("connections", crate::connections::connections_main),
    ("tasks", crate::tasks::tasks_main),
    ("schedule", crate::tasks::schedule_main),
    ("gateway", crate::gateway::serve::main),
];

pub fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if let Some((first, rest)) = words.split_first()
        && let Some((_, subcommand)) = SUBCOMMANDS.iter().find(|(name, _)| name == first)
    {
        return subcommand(rest);
    }
    let args = match args::parse(words) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("glave: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match args.action {
        Action::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Action::Version => {
            println!("glave {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("glave: could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let ran = match args.action {
        Action::Serve => runtime.block_on(serve(args)),
        Action::Explain => runtime.block_on(setup::explain(&args)),
        Action::Skills => runtime.block_on(crate::skills::list()),
        Action::Commands => runtime.block_on(crate::commands::list()),
        Action::App => runtime.block_on(crate::app::main()),
        _ => runtime.block_on(run(args)),
    };
    match ran {
        Ok(code) => code,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The server for this folder, and what the terminal needs to show it.
struct Wired {
    server: Glave,
    servers: Servers,
    /// The default model's endpoint; none while no model is set up.
    endpoint: Option<Endpoint>,
    /// The project of the folder glave runs in, where the terminal's threads are.
    space: SpaceId,
    root: PathBuf,
    /// The instruction files the model is given, for the banner.
    files: Vec<PathBuf>,
    /// The settings in use, for the banner and receipts.
    choices: Choices,
    /// The workers threads work on, with `--on cloud`.
    workers: Option<Arc<WorkerComputers>>,
    /// Memory, when it is on.
    memory: Option<crate::memory::Running>,
    /// Each project's own servers, hooks and settings, and what it asks to be trusted.
    projects: Arc<Projects>,
    /// Logs and telemetry, stopped last.
    _telemetry: crate::telemetry::Telemetry,
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
/// folder: their own, or the catalog of the gateway elsewhere.
async fn mcp_servers(
    setup: &Setup,
    remote: Option<&Remote>,
    processes: &Processes,
) -> anyhow::Result<Servers> {
    Servers::start(
        &home::folder()?,
        &home::person()?,
        setup.choices.mcp.as_ref(),
        &setup.loaded.constraints,
        remote,
        processes,
    )
    .await
}

/// Builds the session server on this computer's store and the current folder.
async fn wire(args: &Args, answering: Answering, asking: Asking) -> anyhow::Result<Wired> {
    let folder = std::env::current_dir()?;
    let mut setup = Setup::load(args, &folder).await?;
    // The terminal asks about the folder's project before glave starts; an app asks later.
    let told = match asking {
        Asking::Later => Vec::new(),
        asking => setup::ask_here(&folder, &setup, asking).await?,
    };
    let telemetry = crate::telemetry::start_for(args.log.as_deref(), &setup.choices)?;
    let database = home::database().await?;
    let processes = home::reaped().await?;
    let workspace = workspace(&folder, &mut setup, &database, &processes).await?;
    let choices = &setup.choices;
    let (remote, configured) = endpoint(choices)?;
    let store = store(&setup).await?;
    let identity = identity::load_or_make(&home::folder()?).await?;
    let lineup = Lineup::new(&setup, configured);
    let lined = line_up(args, lineup, &store, identity.person, remote.is_some()).await?;
    let (workspace, hosts) = with_hosts(args, &store, identity.person, choices, workspace).await?;
    let personal = home::folder()?.join(AGENTS_MD);
    let instructions = agents_md(workspace.root(), Some(&personal)).await?;
    let opening = instructions.as_ref().map(|loaded| loaded.text.clone());
    let mut settings = settings_for(args, choices, &lined, identity, opening)?;
    let model = settings.model.clone();
    let root = workspace.root().to_owned();
    settings.commands = crate::commands::find(&root).await?;
    let servers = mcp_servers(&setup, remote.as_ref(), &processes).await?;
    let (on_workers, workers) =
        crate::cloud::workers(choices, &setup.loaded.constraints, &folder).await?;
    let scope = crate::spaces::here(&store, &root, identity.person).await?;
    let memory = crate::memory::start_for(&store, &setup, identity.person, scope).await?;
    let projects = projects_for(&setup, &servers, &lined.switch, memory.as_ref(), asking)?;
    for line in [told, projects.set_aside(&folder).await].concat() {
        eprintln!("glave: {line}");
    }
    tidy(memory.as_ref(), &lined.switch, model);
    let wiring = Wiring {
        models: lined.built.models.offered,
        modes: policy::thread_modes(choices, &setup.loaded.constraints),
        provider: lined.switch,
        providers: lined.providers,
        git_hosts: hosts.map(|hosts| hosts as Arc<dyn GitHosts>),
        log: store.log(),
        blobs: store.blobs(),
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        workspace,
        web: web(choices)?,
        tools: crate::skills::offered(&root).await?,
        // The person's own servers, for their threads alone.
        own: servers.sources(),
        worktree: choices.worktree.value,
        hooks: Hooks::new(setup.hooks.clone())?,
        workers: on_workers,
        tasks: task_limits(&setup.loaded.constraints),
        memory: memory.as_ref().map(crate::memory::Running::handle),
        spaces: store.spaces(),
        projects: Some(Arc::clone(&projects) as Arc<dyn glave_harness::Projects>),
        trust: Some(Arc::clone(&projects) as Arc<dyn ProjectTrust>),
        browsers: crate::serving::browsers(args)?,
    };
    let server = Server::new(wiring, settings, scope, answering);
    // The folder glave runs in is its project's folder: where its threads work.
    server
        .keep_folder(scope.space, &root.to_string_lossy())
        .await
        .with_context(|| format!("could not keep {} as its project's folder", root.display()))?;
    Ok(Wired {
        server,
        servers,
        endpoint: lined.built.endpoint,
        space: scope.space,
        root,
        files: instructions.map(|loaded| loaded.files).unwrap_or_default(),
        choices: setup.choices,
        workers,
        memory,
        projects,
        _telemetry: telemetry,
    })
}

/// Each project as this glave reads it, beside the `servers` it runs for the person's threads
/// everywhere, offering the models of `models`, with `memory`; asked about from an app when
/// `asking` says the person answers later.
fn projects_for(
    setup: &Setup,
    servers: &Servers,
    models: &Switch,
    memory: Option<&crate::memory::Running>,
    asking: Asking,
) -> anyhow::Result<Arc<Projects>> {
    let own = crate::projects::Own {
        loaded: setup.loaded.clone(),
        choices: setup.choices.clone(),
        taken: servers.names().into_iter().map(str::to_owned).collect(),
        models: models.clone(),
        memory: memory.map(|memory| Arc::clone(&memory.memory)),
        clock: Box::new(SystemClock),
        processes: servers.processes().clone(),
    };
    let later = asking == Asking::Later;
    let projects = Arc::new(Projects::new(home::folder()?, own, later));
    Projects::tidy(&projects);
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
            let asked = Arc::clone(&hosts) as Arc<dyn Credentials>;
            (workspace.with_credentials(asked), Some(hosts))
        }
        None => (workspace, None),
    })
}

/// Tidies memory in the background, when it is on: tidy-up asks the gateway too, which sends
/// it on the `summary` route.
fn tidy(memory: Option<&crate::memory::Running>, switch: &Switch, model: String) {
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
    switch: Switch,
    /// The person's model accounts, when they may connect them here.
    providers: Option<Arc<dyn Providers>>,
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
    let ledger = if through_gateway || !args.issuers.is_empty() {
        None
    } else {
        Some(Ledger::open(store, person).await?)
    };
    let accounts = match &ledger {
        Some(ledger) => ledger.accounts().await?.providers,
        None => Vec::new(),
    };
    // An account that no longer lines up with the settings never stops glave starting.
    let mut built = match lineup.build(&accounts) {
        Ok(built) => built,
        Err(error) if !accounts.is_empty() => {
            eprintln!("glave: the model accounts you connected are set aside: {error:#}");
            lineup.build(&[])?
        }
        Err(error) => return Err(error),
    };
    let switch = Switch::new(built.gateway.take());
    let providers = ledger
        .map(|ledger| Arc::new(Host::new(ledger, lineup, switch.clone())) as Arc<dyn Providers>);
    Ok(Lined {
        built,
        switch,
        providers,
    })
}

/// The gateway elsewhere, when `[gateway] url` names one, and the endpoint model calls go to:
/// that gateway, or the model chosen, if any is.
fn endpoint(choices: &Choices) -> anyhow::Result<(Option<Remote>, Option<Endpoint>)> {
    let remote = choices
        .gateway
        .as_ref()
        .map(Remote::from_choice)
        .transpose()?;
    let model = choices.model.as_ref().map(|m| m.value.clone());
    let endpoint = match (&remote, model) {
        (Some(remote), model) => Some(remote.endpoint(model)?),
        (None, Some(model)) => Some(config::endpoint(Some(model), choices.provider.as_ref())?),
        (None, None) => None,
    };
    let idle = config::idle_timeout()?;
    Ok((
        remote,
        endpoint.map(|mut endpoint| {
            if let Some(idle) = idle {
                endpoint.provider.idle_timeout = idle;
            }
            endpoint
        }),
    ))
}

/// `web_fetch`, as `[web]` sets it up; nothing when it is off.
fn web(choices: &Choices) -> anyhow::Result<Option<Arc<dyn ToolSource>>> {
    let web = crate::projects::web_fetch(&choices.web.value)?;
    Ok(web
        .offers_any()
        .then(|| Arc::new(web) as Arc<dyn ToolSource>))
}

/// The data store the setup's choices name, once this machine's worktrees have had their
/// daily check.
async fn store(setup: &Setup) -> anyhow::Result<Data> {
    let store = Data::chosen(&setup.choices).await?;
    crate::worktrees::check_at_start(&store.log()).await;
    Ok(store)
}

/// The workspace in `folder`, in the sandbox the setup's choices ask for. Where glave's
/// default sandbox cannot run, commands go unconfined and the choices say so: approval
/// becomes `ask` (`policy::unsandboxed`), unless threads work on workers, whose own
/// container or virtual machine is their wall. A sandbox someone chose, or a policy
/// requires, is never given up: glave refuses to start instead.
async fn workspace(
    folder: &Path,
    setup: &mut Setup,
    database: &Path,
    processes: &Processes,
) -> anyhow::Result<Workspace> {
    let choices = &setup.choices;
    let Some(level) = choices.sandbox.value else {
        return open_here(folder, processes).await;
    };
    let sandbox = Sandbox {
        level,
        network: choices.network.value,
        home: std::env::var_os("HOME").map(PathBuf::from),
        glave_home: database.parent().map(Path::to_path_buf),
    };
    let error = match open_here(folder, processes)
        .await?
        .with_sandbox(&sandbox)
        .await
    {
        Ok(workspace) => return Ok(workspace),
        Err(error) => error,
    };
    if choices.sandbox.from != Source::Default {
        return Err(error).with_context(|| {
            format!(
                "the sandbox was asked for (by {}), so glave will not run commands without it",
                choices.sandbox.from
            )
        });
    }
    let why = error.to_string();
    let approval = choices.approval.clone();
    let constraints = &setup.loaded.constraints;
    setup.choices = policy::unsandboxed(setup.choices.clone(), &why, constraints);
    if setup.choices.on.value == On::Cloud {
        setup.choices.approval = approval;
    } else {
        eprintln!("glave: {}", unsandboxed_notice(&setup.choices, &why));
    }
    open_here(folder, processes).await
}

/// The workspace in `folder` on this computer, without a sandbox yet.
async fn open_here(folder: &Path, processes: &Processes) -> anyhow::Result<Workspace> {
    let computer = identity::computer(&home::folder()?).await?;
    let computers = LocalComputers::for_here(std::env::vars_os())
        .known_as(computer)
        .with_worktrees(home::folder()?.join(crate::worktrees::FOLDER))
        .with_scratch(home::folder()?.join(crate::worktrees::SCRATCH))
        .with_projects(home::projects()?)
        .with_processes(processes.clone());
    let (computers, here) = crate::computer::with_sandbox_helper(computers)
        .open_here(folder)
        .await?;
    Ok(Workspace::new(computers, here))
}

/// What the person is told, once at startup, when glave's default sandbox cannot run here.
fn unsandboxed_notice(choices: &Choices, why: &str) -> String {
    let asks = if choices.approval.value == ApprovalMode::Ask {
        "so every command and change asks you first (approval: ask)"
    } else {
        "and the policy allows only approval auto, so commands run as you without asking"
    };
    format!(
        "the sandbox cannot run on this computer ({why}), {asks}. To run commands without \
         the sandbox and without asking, choose both sandbox = \"off\" and approval = \"auto\" \
         (--sandbox off --approval auto)."
    )
}

async fn run(args: Args) -> anyhow::Result<ExitCode> {
    if let Some(url) = crate::remote::chosen(&args).await? {
        return crate::remote::run(&args, &url).await;
    }
    if args.background {
        return crate::tasks::background(&args).await;
    }
    // Ask in the terminal when there is one; otherwise no one is there to answer.
    let asks = std::io::stdin().is_terminal();
    let answering = if asks {
        Answering::Clients
    } else {
        Answering::NoOne
    };
    let wired = wire(
        &args,
        answering,
        if asks { Asking::Person } else { Asking::NoOne },
    )
    .await?;
    // A first run has no model yet: the browser is where one is added.
    let screen = crate::app::screen_here();
    let opens_app = crate::app::first_run(&args, asks, screen, wired.endpoint.is_some());
    let ran = if opens_app {
        Ok(ExitCode::SUCCESS)
    } else {
        Box::pin(session(&args, &wired, asks)).await
    };
    wired.server.shutdown().await;
    if let Some(workers) = &wired.workers {
        crate::cloud::release(&wired.server, workers).await;
    }
    wired.servers.close().await;
    wired.projects.close().await;
    close_memory(&wired).await;
    if opens_app {
        eprintln!("glave: no model is set up yet, so glave opens in your browser to add one");
        return crate::app::main().await;
    }
    ran
}

/// Serves the session API: to the program that started glave until its input ends, as a
/// folder's task host until it is idle, or as the person's server until it is told to stop.
pub async fn serve(mut args: Args) -> anyhow::Result<ExitCode> {
    let lock = crate::server::claim(&mut args).await?; // Held to the very end.
    // The session API owns standard input: a project is asked about through it, in an app.
    let wired = wire(&args, Answering::Clients, Asking::Later).await?;
    let served = if args.socket {
        let folder = std::env::current_dir()?;
        crate::daemon::host(&wired.server, &crate::daemon::socket(&folder)?).await
    } else if let Some(lock) = &lock {
        crate::server::run(&args, &wired.server, lock).await
    } else {
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        glave_session::rpc::serve(wired.server.clone(), input, tokio::io::stdout())
            .await
            .map_err(anyhow::Error::from)
    };
    if args.socket || wired.workers.is_some() {
        wired.server.shutdown().await;
    }
    if let Some(workers) = &wired.workers {
        crate::cloud::release(&wired.server, workers).await;
    }
    wired.servers.close().await;
    wired.projects.close().await;
    close_memory(&wired).await;
    drop(lock); // Last, once everything it ran has stopped.
    served?;
    Ok(ExitCode::SUCCESS)
}

/// Picks the thread, runs the turn or the session, and exports the events when asked.
async fn session(args: &Args, wired: &Wired, asks: bool) -> anyhow::Result<ExitCode> {
    let Wired {
        server,
        servers,
        endpoint,
        space,
        root,
        files,
        choices,
        workers: _,
        memory: _,
        projects,
        _telemetry: _,
    } = wired;
    let Some(endpoint) = endpoint else {
        anyhow::bail!(config::NO_MODEL);
    };
    // The terminal's threads are the folder's project's, so it shows what that chooses.
    let project = projects.settled(root).await;
    let model = project.model.unwrap_or_else(|| endpoint.model.clone());
    crate::tasks::at_start(server, args).await;
    let here = Session::Here(server);
    let (thread, note) = pick_thread(here, Some(*space), args.resume, None, None, None).await?;
    if let Some(working) = crate::worktrees::note(server, thread).await {
        eprintln!("glave: {working}");
    }
    let input = Input::start();
    let client = Client {
        server: here,
        thread,
        input: &input,
        endpoint: Some(endpoint),
        asks,
        servers: Some(servers),
        model: None,
        approval: None,
    };
    let mut printer = Printer::new(&model, protections(choices, project.protections));
    let code = if let Some(objective) = &args.goal {
        let goal = Goal {
            objective: objective.clone(),
            token_budget: args.goal_budget.unwrap_or(DEFAULT_BUDGET),
        };
        client.goal(goal, &mut printer).await
    } else if args.prompt.is_empty() {
        let mut note = match named(files) {
            Some(files) => format!("{note} · following {files}"),
            None => note.to_owned(),
        };
        let names = servers.names();
        if !names.is_empty() {
            note = format!("{note} · MCP: {}", names.join(", "));
        }
        let folder = root.display().to_string();
        printer.banner(&model, &endpoint.provider.provider, &folder, &note);
        converse(&client, &mut printer).await
    } else {
        client.turn(&args.prompt, &mut printer).await
    };
    if let Some(path) = &args.events {
        export(here, thread, path).await?;
    }
    Ok(code)
}

/// The thread to use in `space` (the caller's Chats when none): the newest one there with
/// `--continue`, otherwise a new one, in a worktree of its own when `worktree` says so (the
/// server's choice when it says nothing), choosing `model` and `approval` when they are given.
pub async fn pick_thread(
    server: Session<'_>,
    space: Option<SpaceId>,
    resume: bool,
    worktree: Option<bool>,
    model: Option<String>,
    approval: Option<ApprovalMode>,
) -> anyhow::Result<(ThreadId, &'static str)> {
    if resume {
        let latest = match server
            .request(Request::ThreadList {
                space,
                limit: Some(1),
                show: None,
                cursor: None,
            })
            .await?
        {
            Reply::Threads { threads, .. } => threads.first().map(|thread| thread.id),
            _ => None,
        };
        if let Some(thread) = latest {
            // Loading it closes anything a crash left open.
            server.request(Request::ThreadGet { thread }).await?;
            return Ok((thread, "continuing your last thread"));
        }
    }
    let note = if resume {
        "no earlier thread here; new thread"
    } else {
        "new thread"
    };
    match server
        .request(Request::ThreadStart {
            space,
            background: false,
            worktree,
            computer: None,
            model,
            approval,
        })
        .await?
    {
        Reply::Thread { thread } => Ok((thread.id, note)),
        other => anyhow::bail!("the server answered thread/start with {other:?}"),
    }
}

/// The interactive session: each line typed is a turn, until `/exit`, the end of input, or
/// Ctrl-C at the prompt. Ctrl-C during a turn or `/compact` cancels just that.
pub async fn converse(client: &Client<'_>, printer: &mut Printer) -> ExitCode {
    let mut code = ExitCode::SUCCESS;
    loop {
        printer.ask();
        let interrupt = Cancel::new();
        let typed = {
            let _ctrl_c = CtrlC::cancels(&interrupt);
            interrupt.run(client.input.line()).await
        };
        let Some(Some(line)) = typed else {
            println!();
            break;
        };
        let line = line.trim();
        if line == "/exit" {
            break;
        }
        if line == "/compact" {
            client.compact(printer).await;
            continue;
        }
        if line == "/memory" {
            show_memory(client.server).await;
            continue;
        }
        if !line.is_empty() {
            code = client.turn(line, printer).await;
        }
    }
    code
}

/// Batches the session's forgets into a space-key rotation, then stops memory.
async fn close_memory(wired: &Wired) {
    if let Some(memory) = &wired.memory {
        memory.housekeep().await;
        memory.memory.close().await;
    }
}

/// `/memory`: the memories this session's threads read, through the session API.
async fn show_memory(server: Session<'_>) {
    match server.request(Request::MemoryList { scope: None }).await {
        Ok(Reply::Memories { memories }) if memories.is_empty() => println!("no memories here"),
        Ok(Reply::Memories { memories }) => {
            for memory in &memories {
                println!("{}", crate::memory::line(memory));
            }
            println!("change them with: glave memory edit|move|forget <id>");
        }
        Ok(_) => {}
        Err(error) => println!("{error}"),
    }
}

/// Writes the thread's events as JSON Lines, one event per line, exactly as stored.
pub async fn export(server: Session<'_>, thread: ThreadId, path: &Path) -> anyhow::Result<()> {
    let mut lines = String::new();
    let mut after = None;
    loop {
        let page = Request::ThreadEvents {
            thread,
            after,
            limit: Some(5_000),
        };
        let Reply::Events { events, next } = server.request(page).await? else {
            anyhow::bail!("the server answered thread/events with something else");
        };
        for event in &events {
            lines.push_str(&event.to_json()?);
            lines.push('\n');
        }
        match next {
            Some(seq) => after = Some(seq),
            None => break,
        }
    }
    tokio::fs::write(path, lines).await?;
    Ok(())
}

/// The instruction files for the banner, with the home folder as `~`:
/// `~/.glave/AGENTS.md, ~/code/app/AGENTS.md`.
fn named(files: &[PathBuf]) -> Option<String> {
    let names: Vec<String> = files
        .iter()
        .map(|file| {
            std::env::var_os("HOME")
                .and_then(|home| file.strip_prefix(home).ok())
                .map_or_else(
                    || file.display().to_string(),
                    |rest| format!("~/{}", rest.display()),
                )
        })
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// The protections commands run under, as the folder's project changes them, for the banner
/// and receipts: `approval: auto · sandbox: project`.
fn protections(choices: &Choices, project: Protections) -> String {
    let approval = project.mode(None, choices.approval.value);
    let approval = format!("approval: {}", approval.name());
    let own = choices.sandbox.value.map(|level| Sandbox {
        level,
        network: choices.network.value,
        home: None,
        glave_home: None,
    });
    match project.sandbox(own.as_ref()) {
        Some(sandbox) => {
            let network = if sandbox.network { " + network" } else { "" };
            format!("{approval} · sandbox: {}{network}", sandbox.level.name())
        }
        None => approval,
    }
}
