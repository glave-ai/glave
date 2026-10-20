//! glave's entry: its commands, one turn or an interactive session in the terminal as the
//! session server's client (wired in `wiring.rs`), and serving the session API.

use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::process::ExitCode;

use glave_config::policy::Choices;
use glave_config::trust::Asking;
use glave_event::ThreadId;
use glave_harness::{ApprovalMode, Cancel, DEFAULT_BUDGET, Goal, ProjectFolders as _};
use glave_ports::ProjectId;
use glave_ports::computer::Sandbox;
use glave_ports::settings::Protections;
use glave_session::Answering;
use glave_session::protocol::{Reply, Request};

use crate::args::{self, Action, Args, USAGE};
use crate::client::{Client, Session};
use crate::config;
use crate::input::{CtrlC, Input};
use crate::setup;
use crate::terminal::Printer;
use crate::wiring::Wired;

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

pub(crate) fn main() -> ExitCode {
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

async fn run(args: Args) -> anyhow::Result<ExitCode> {
    if let Some(url) = crate::remote::chosen(&args).await? {
        return crate::remote::run(&args, &url).await;
    }
    if args.background {
        return crate::tasks::background(&args).await;
    }
    // A turn or a goal goes to the person's server, started from the person's own terminal;
    // where none runs and no one is at the terminal, a server here answers it, asking no one.
    let turn = !args.prompt.is_empty() || args.goal.is_some();
    if turn && let Some(server) = crate::server::connect_or_start().await? {
        let terminal = crate::server::at_terminal();
        return Box::pin(crate::turns::run(&args, &server, terminal)).await;
    }
    // The session in the terminal asks there; a turn here has no one to answer it.
    let asks = !turn && std::io::stdin().is_terminal();
    let answering = if asks {
        Answering::Clients
    } else {
        Answering::NoOne
    };
    let wired = crate::wiring::wire(
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
    wired.close().await;
    if opens_app {
        eprintln!("glave: no model is set up yet, so glave opens in your browser to add one");
        return crate::app::main().await;
    }
    ran
}

/// Serves the session API: to the program that started glave until its input ends, or as the
/// person's server until it is told to stop.
pub(crate) async fn serve(mut args: Args) -> anyhow::Result<ExitCode> {
    let lock = crate::server::claim(&mut args).await?; // Held to the very end.
    // The session API owns standard input: a project is asked about through it, in an app.
    let wired = crate::wiring::wire(&args, Answering::Clients, Asking::Later).await?;
    let served = if let Some(lock) = &lock {
        let daily = crate::server::Daily {
            log: wired.log.clone(),
            memory: wired.memory.as_ref(),
        };
        crate::server::run(&args, &wired.server, &daily, lock).await
    } else {
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        glave_session::rpc::serve(wired.server.clone(), input, tokio::io::stdout())
            .await
            .map_err(anyhow::Error::from)
    };
    if wired.workers.is_some() {
        wired.server.shutdown().await;
    }
    wired.close().await;
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
        project,
        root,
        files,
        choices,
        workers: _,
        memory: _,
        log: _,
        projects,
        _telemetry: _,
    } = wired;
    let Some(endpoint) = endpoint else {
        anyhow::bail!(config::NO_MODEL);
    };
    // The terminal's threads are the folder's project's, so it shows what that chooses.
    let settled = projects.settled(root).await;
    let model = settled.model.unwrap_or_else(|| endpoint.model.clone());
    let here = Session::Here(server);
    let (thread, note) = pick_thread(here, Some(*project), args.resume, None, None, None).await?;
    if let Some(working) = crate::worktrees::note(here, thread).await {
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
    let mut printer = Printer::new(&model, protections(choices, settled.protections));
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
        crate::client::export(here, thread, path).await?;
    }
    Ok(code)
}

/// The thread to use in `project` (the caller's Chats when none): the newest one there with
/// `--continue`, otherwise a new one, in a worktree of its own when `worktree` says so (the
/// server's choice when it says nothing), choosing `model` and `approval` when they are given.
pub(crate) async fn pick_thread(
    server: Session<'_>,
    project: Option<ProjectId>,
    resume: bool,
    worktree: Option<bool>,
    model: Option<String>,
    approval: Option<ApprovalMode>,
) -> anyhow::Result<(ThreadId, &'static str)> {
    if resume {
        let latest = match server
            .request(Request::ThreadList {
                project,
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
            project,
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
pub(crate) async fn converse(client: &Client<'_>, printer: &mut Printer) -> ExitCode {
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

/// `/memory`: the memories this session's threads read, through the session API.
async fn show_memory(server: Session<'_>) {
    match server
        .request(Request::MemoryList {
            scope: None,
            project: None,
        })
        .await
    {
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
