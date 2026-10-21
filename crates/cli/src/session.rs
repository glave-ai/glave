//! glave's entry: its commands; `glave` alone opening the browser (`app.rs`); a turn or a goal
//! through the person's server (`turns.rs`), or in this command's own process where none runs
//! and no one is at the terminal (wired in `wiring.rs`); and serving the session API.

use std::process::ExitCode;

use glave_config::policy::Choices;
use glave_config::trust::Asking;
use glave_event::ThreadId;
use glave_harness::{ApprovalMode, DEFAULT_BUDGET, Goal, ProjectFolders as _};
use glave_ports::ProjectId;
use glave_ports::computer::Sandbox;
use glave_ports::settings::Protections;
use glave_session::Answering;
use glave_session::protocol::{Reply, Request};

use crate::args::{self, Action, Args, USAGE};
use crate::client::{Client, Session};
use crate::config;
use crate::input::Input;
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
    // Watched for as long as glave runs, whatever it runs as.
    let _starter = match crate::starter::watch() {
        Ok(watching) => watching,
        Err(message) => {
            eprintln!("glave: {message}");
            return ExitCode::FAILURE;
        }
    };
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
    let turn = !args.prompt.is_empty() || args.goal.is_some();
    if let Some(url) = crate::remote::chosen(&args).await? {
        if !turn {
            anyhow::bail!(
                "glave alone opens glave on this computer in a browser; for the server at {url}, \
                 give it a prompt, or open its address in a browser"
            );
        }
        return crate::remote::run(&args, &url).await;
    }
    if args.background {
        return crate::tasks::background(&args).await;
    }
    if !turn {
        return crate::app::open(&args).await;
    }
    // A turn or a goal goes to the person's server, started from the person's own terminal;
    // where none runs and no one is at the terminal, a server here answers it, asking no one.
    if let Some(server) = crate::server::connect_or_start().await? {
        let terminal = crate::server::at_terminal();
        return Box::pin(crate::turns::run(&args, &server, terminal)).await;
    }
    let wired = crate::wiring::wire(&args, Answering::NoOne, Asking::NoOne).await?;
    let ran = Box::pin(here(&args, &wired)).await;
    wired.server.shutdown().await;
    wired.close().await;
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
            blobs: wired.blobs.clone(),
            org: wired.org,
            memory: wired.memory.as_ref(),
        };
        crate::server::run(&args, &wired.reach, &wired.server, &daily, lock).await
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

/// Runs the turn or goal in this command's own process, answered by no one, and exports the
/// events when asked.
async fn here(args: &Args, wired: &Wired) -> anyhow::Result<ExitCode> {
    let Wired {
        server,
        servers,
        endpoint,
        project,
        root,
        choices,
        projects,
        ..
    } = wired;
    let Some(endpoint) = endpoint else {
        anyhow::bail!(config::NO_MODEL);
    };
    // The turn is the folder's project's, so it shows what that chooses.
    let settled = projects.settled(root).await;
    let model = settled.model.unwrap_or_else(|| endpoint.model.clone());
    let here = Session::Here(server);
    let (thread, _) = pick_thread(here, Some(*project), args.resume, None, None, None).await?;
    if let Some(working) = crate::worktrees::note(here, thread).await {
        eprintln!("glave: {working}");
    }
    let input = Input::start();
    let client = Client {
        server: here,
        thread,
        input: &input,
        endpoint: Some(endpoint),
        asks: false,
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
            effort: None,
        })
        .await?
    {
        Reply::Thread { thread } => Ok((thread.id, note)),
        other => anyhow::bail!("the server answered thread/start with {other:?}"),
    }
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
