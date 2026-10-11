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
    AGENTS_MD, ApprovalMode, Cancel, DEFAULT_BUDGET, Goal, Hooks, OsEntropy, Sandbox, Settings,
    SystemClock, Workspace, agents_md,
};
use glave_ports::ToolSource;
use glave_session::protocol::{Reply, Request};
use glave_session::{Answering, Server, Wiring};
use glave_store_sqlite::Store;
use glave_web::{Web, WebSettings};

use crate::args::{self, Action, Args, USAGE};
use crate::client::{Client, Glave};
use crate::config::{self, Endpoint};
use crate::gateway::Allowed;
use crate::gateway::remote::Remote;
use crate::input::{CtrlC, Input};
use crate::mcp::Servers;
use crate::model::Model;
use crate::setup::{self, Setup};
use crate::terminal::Printer;
use crate::{gateway, home, identity, keys};
use glave_config::policy::{self, Choices, On, Source};
use glave_config::trust::{self, Asking};
use glave_gateway::Gateway;

pub fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    // `computer` is a subcommand only as the first word; a prompt starting with it goes
    // after `--`.
    if let Some((first, rest)) = words.split_first()
        && first == "computer"
    {
        return crate::computer::main(rest);
    }
    if let Some((first, rest)) = words.split_first()
        && first == "worktrees"
    {
        return crate::worktrees::main(rest);
    }
    if let Some((first, rest)) = words.split_first()
        && first == "threads"
    {
        return crate::threads::main(rest);
    }
    if let Some((first, rest)) = words.split_first()
        && first == "memory"
    {
        return crate::memory::main(rest);
    }
    if let Some((first, rest)) = words.split_first()
        && first == "tasks"
    {
        return crate::tasks::tasks_main(rest);
    }
    if let Some((first, rest)) = words.split_first()
        && first == "schedule"
    {
        return crate::tasks::schedule_main(rest);
    }
    if let Some((first, rest)) = words.split_first()
        && first == "gateway"
    {
        return crate::gateway::serve::main(rest);
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
        Action::Run | Action::Serve | Action::Explain | Action::Skills | Action::Commands => {}
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
    endpoint: Endpoint,
    root: PathBuf,
    /// The instruction files the model is given, for the banner.
    files: Vec<PathBuf>,
    /// The settings in use, for the banner and receipts.
    choices: Choices,
    /// The workers threads work on, with `--on cloud`.
    workers: Option<Arc<WorkerComputers>>,
    /// Memory, when it is on.
    memory: Option<crate::memory::Running>,
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

/// Builds the session server on this computer's store and the current folder.
async fn wire(args: &Args, answering: Answering, asking: Asking) -> anyhow::Result<Wired> {
    let folder = std::env::current_dir()?;
    let mut setup = Setup::load(args, &folder, asking).await?;
    if let Some(notice) = setup.notice() {
        eprintln!("glave: {notice}");
    }
    let telemetry = crate::telemetry::start_for(args.log.as_deref(), &setup.choices)?;
    let database = home::database().await?;
    let workspace = workspace(&folder, &mut setup, &database).await?;
    let choices = &setup.choices;
    let (remote, endpoint) = endpoint(choices)?;
    let (provider, judge) = provider(&setup, &endpoint)?;
    let store = store(database).await?;
    let mut settings = Settings::new(endpoint.model.clone(), endpoint.provider.provider.clone());
    let personal = home::folder()?.join(AGENTS_MD);
    let instructions = agents_md(workspace.root(), Some(&personal)).await?;
    settings.instructions = [
        instructions.as_ref().map(|loaded| loaded.text.clone()),
        setup.model_note(),
    ]
    .into_iter()
    .flatten()
    .reduce(|all, part| format!("{all}\n\n{part}"));
    settings.approval = choices.approval.value;
    settings.max_steps = args.max_steps;
    settings.subagents = true; // As Claude Code, Codex and OpenCode offer it.
    settings.judge = judge.is_some();
    let identity = identity::load_or_make(&home::folder()?).await?;
    (settings.person, settings.agent) = (Some(identity.person), Some(identity.agent));
    let window = config::context_window()?.unwrap_or(settings.compaction.window_tokens);
    settings.compaction.window_tokens = window;
    let root = workspace.root().to_owned();
    settings.commands = crate::commands::find(&root).await?;
    let servers = Servers::start(
        &home::folder()?,
        &root,
        choices.mcp.as_ref(),
        &setup.loaded.constraints,
        (!setup.project_servers.is_empty())
            .then(|| trust::mcp_json(&setup.project_servers))
            .as_deref(),
        remote.as_ref(),
    )
    .await?;
    // The task host puts tasks on workers whenever a backend is set up.
    let (on_workers, workers) =
        crate::cloud::workers(choices, &setup.loaded.constraints, &folder, args.socket).await?;
    let scope = crate::spaces::here(&store, &root, identity.person).await?;
    let memory = crate::memory::start_for(&store, &setup, identity.person, scope).await?;
    let wiring = Wiring {
        provider,
        log: store.log(),
        blobs: store.blobs(),
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        workspace,
        // web_fetch first, then the project's skills, then the MCP servers' tools.
        tools: [
            web(choices)?,
            crate::skills::offered(&root).await?,
            servers.sources(),
        ]
        .concat(),
        worktree: choices.worktree.value,
        hooks: Hooks::new(setup.hooks.clone())?,
        workers: on_workers,
        tasks: task_limits(&setup.loaded.constraints),
        memory: memory.as_ref().map(crate::memory::Running::handle),
        spaces: Arc::new(store.spaces()),
    };
    Ok(Wired {
        server: Server::new(wiring, settings, scope, answering),
        servers,
        endpoint,
        root,
        files: instructions.map(|loaded| loaded.files).unwrap_or_default(),
        choices: setup.choices,
        workers,
        memory,
        _telemetry: telemetry,
    })
}

/// The gateway elsewhere, when `[gateway] url` names one, and the endpoint model calls go to:
/// that gateway, or the model chosen.
fn endpoint(choices: &Choices) -> anyhow::Result<(Option<Remote>, Endpoint)> {
    let remote = choices
        .gateway
        .as_ref()
        .map(Remote::from_choice)
        .transpose()?;
    let model = choices.model.as_ref().map(|m| m.value.clone());
    let mut endpoint = match &remote {
        Some(remote) => remote.endpoint(model)?,
        None => config::endpoint(model)?,
    };
    if let Some(idle) = config::idle_timeout()? {
        endpoint.provider.idle_timeout = idle;
    }
    Ok((remote, endpoint))
}

/// `web_fetch`, as `[web]` sets it up; nothing when it is off.
fn web(choices: &Choices) -> anyhow::Result<Vec<Arc<dyn ToolSource>>> {
    let settings = WebSettings {
        policy: choices.web.value.policy()?,
        limits: glave_web::Limits::default(),
    };
    let web = Web::new(settings).context("could not set up web_fetch")?;
    Ok(if web.offers_any() {
        vec![Arc::new(web)]
    } else {
        Vec::new()
    })
}

/// The gateway to the chosen model, and the judge model, if any.
fn provider(setup: &Setup, endpoint: &Endpoint) -> anyhow::Result<(Gateway, Option<String>)> {
    let choices = &setup.choices;
    let judge = choices.judge.value.model(&endpoint.model);
    let allowed = Allowed {
        constraints: &setup.loaded.constraints,
        main: choices.model.as_ref().map(|m| &m.from),
        judge: &choices.judge.from,
    };
    let provider = gateway::gateway(
        Model::for_endpoint(&endpoint.provider)?,
        endpoint,
        judge.as_deref(),
        &allowed,
    )?;
    Ok((provider, judge))
}

/// The store in `database`, once this machine's worktrees have had their daily check.
async fn store(database: PathBuf) -> anyhow::Result<Store> {
    let keys = keys::settings(&home::folder()?, home::relocated()).await?;
    let store = Store::open(database, keys).await?;
    crate::worktrees::check_at_start(&store.log()).await;
    Ok(store)
}

/// The workspace in `folder`, in the sandbox the setup's choices ask for. Where glave's
/// default sandbox cannot run, commands go unconfined and the choices say so: approval
/// becomes `ask` (`policy::unsandboxed`), unless threads work on workers, whose own
/// container or virtual machine is their wall. A sandbox someone chose, or a policy
/// requires, is never given up: glave refuses to start instead.
async fn workspace(folder: &Path, setup: &mut Setup, database: &Path) -> anyhow::Result<Workspace> {
    let choices = &setup.choices;
    let Some(level) = choices.sandbox.value else {
        return open_here(folder).await;
    };
    let sandbox = Sandbox {
        level,
        network: choices.network.value,
        home: std::env::var_os("HOME").map(PathBuf::from),
        glave_home: database.parent().map(Path::to_path_buf),
    };
    let error = match open_here(folder).await?.with_sandbox(&sandbox).await {
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
    open_here(folder).await
}

/// The workspace in `folder` on this computer, without a sandbox yet.
async fn open_here(folder: &Path) -> anyhow::Result<Workspace> {
    let computers = LocalComputers::for_here(std::env::vars_os())
        .with_worktrees(home::folder()?.join(crate::worktrees::FOLDER))
        .with_scratch(home::folder()?.join(crate::worktrees::SCRATCH));
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
    let ran = session(&args, &wired, asks).await;
    wired.server.shutdown().await;
    if let Some(workers) = &wired.workers {
        crate::cloud::release(&wired.server, workers).await;
    }
    wired.servers.close().await;
    close_memory(&wired).await;
    ran
}

/// Serves the session API: to the program that started glave until its input ends, or over
/// HTTP until Ctrl-C.
async fn serve(args: Args) -> anyhow::Result<ExitCode> {
    // The session API owns standard input, so there is no one to ask about a project.
    let wired = wire(&args, Answering::Clients, Asking::NoOne).await?;
    let served = if args.socket {
        let folder = std::env::current_dir()?;
        crate::daemon::host(&wired.server, &crate::daemon::socket(&folder)?).await
    } else if let Some(address) = args.http {
        crate::serving::serve_http(&args, wired.server.clone(), address).await
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
    close_memory(&wired).await;
    served?;
    Ok(ExitCode::SUCCESS)
}

/// Picks the thread, runs the turn or the session, and exports the events when asked.
async fn session(args: &Args, wired: &Wired, asks: bool) -> anyhow::Result<ExitCode> {
    let Wired {
        server,
        servers,
        endpoint,
        root,
        files,
        choices,
        workers: _,
        memory: _,
        _telemetry: _,
    } = wired;
    crate::tasks::at_start(server, args).await;
    let (thread, note) = pick_thread(server, args.resume).await?;
    if let Some(working) = crate::worktrees::note(server, thread).await {
        eprintln!("glave: {working}");
    }
    let input = Input::start();
    let client = Client {
        server,
        thread,
        input: &input,
        endpoint,
        asks,
        servers,
    };
    let mut printer = Printer::new(&endpoint.model, protections(choices));
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
        printer.banner(&endpoint.model, &endpoint.provider.provider, &folder, &note);
        converse(&client, &mut printer).await
    } else {
        client.turn(&args.prompt, &mut printer).await
    };
    if let Some(path) = &args.events {
        export(server, thread, path).await?;
    }
    Ok(code)
}

/// The thread to use: the newest one here with `--continue`, otherwise a new one.
async fn pick_thread(server: &Glave, resume: bool) -> anyhow::Result<(ThreadId, &'static str)> {
    if resume {
        let latest = match server
            .request(Request::ThreadList {
                space: None,
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
            space: None,
            background: false,
            worktree: None,
        })
        .await?
    {
        Reply::Thread { thread } => Ok((thread.id, note)),
        other => anyhow::bail!("the server answered thread/start with {other:?}"),
    }
}

/// The interactive session: each line typed is a turn, until `/exit`, the end of input, or
/// Ctrl-C at the prompt. Ctrl-C during a turn or `/compact` cancels just that.
async fn converse(client: &Client<'_>, printer: &mut Printer) -> ExitCode {
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
async fn show_memory(server: &Glave) {
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
async fn export(server: &Glave, thread: ThreadId, path: &Path) -> anyhow::Result<()> {
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

/// The protections commands run under, for the banner and receipts: `sandbox: project`.
fn protections(choices: &Choices) -> String {
    let approval = format!("approval: {}", choices.approval.value.name());
    match choices.sandbox.value {
        Some(level) => {
            let network = if choices.network.value {
                " + network"
            } else {
                ""
            };
            format!("{approval} · sandbox: {}{network}", level.name())
        }
        None => approval,
    }
}
