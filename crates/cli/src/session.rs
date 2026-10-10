//! Wiring glave together: the session server on this computer's store and workspace, with
//! the terminal as its client, for one turn or an interactive session.

use std::io::IsTerminal as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use glave_computer::LocalComputers;
use glave_event::ThreadId;
use glave_harness::{
    AGENTS_MD, Cancel, OsEntropy, Sandbox, Settings, SystemClock, Workspace, agents_md,
};
use glave_session::protocol::{Reply, Request};
use glave_session::signin::{SignIn, Trust};
use glave_session::{Answering, Server, Wiring};
use glave_store_sqlite::Store;
use tokio::net::TcpListener;

use crate::args::{self, Action, Args, USAGE};
use crate::client::{Client, Glave};
use crate::config::{self, Endpoint};
use crate::input::{CtrlC, Input};
use crate::mcp::Servers;
use crate::model::Model;
use crate::system::random_uuid;
use crate::terminal::Printer;
use crate::{gateway, home, identity, keys};

pub fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    // `computer` is a subcommand only as the first word; a prompt starting with it goes
    // after `--`.
    if let Some((first, rest)) = words.split_first()
        && first == "computer"
    {
        return crate::computer::main(rest);
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
        Action::Run | Action::Serve => {}
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
    let ran = if args.action == Action::Serve {
        runtime.block_on(serve(args))
    } else {
        runtime.block_on(run(args))
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
}

/// Builds the session server on this computer's store and the current folder.
async fn wire(args: &Args, answering: Answering) -> anyhow::Result<Wired> {
    let mut endpoint = config::endpoint(args.model.clone())?;
    if let Some(idle) = config::idle_timeout()? {
        endpoint.provider.idle_timeout = idle;
    }
    let judge = config::judge_model(&endpoint.model);
    let provider = gateway::gateway(
        Model::for_endpoint(&endpoint.provider)?,
        &endpoint,
        judge.as_deref(),
    )?;
    let (computers, here) =
        LocalComputers::here(std::env::current_dir()?, std::env::vars_os()).await?;
    let mut workspace = Workspace::new(computers, here);
    let database = home::database().await?;
    if let Some(level) = args.sandbox {
        let sandbox = Sandbox {
            level,
            network: args.network,
            home: std::env::var_os("HOME").map(PathBuf::from),
            glave_home: database.parent().map(Path::to_path_buf),
        };
        workspace = workspace
            .with_sandbox(&sandbox)
            .await
            .context("--sandbox was asked for, so glave will not run commands without it")?;
    }
    let keys = keys::settings(&home::folder()?, home::relocated()).await?;
    let store = Store::open(database, keys).await?;
    let mut settings = Settings::new(endpoint.model.clone(), endpoint.provider.provider.clone());
    let personal = home::folder()?.join(AGENTS_MD);
    let instructions = agents_md(workspace.root(), Some(&personal)).await?;
    settings.instructions = instructions.as_ref().map(|loaded| loaded.text.clone());
    settings.approval = args.approval.unwrap_or_default();
    settings.max_steps = args.max_steps;
    // As Claude Code, Codex and OpenCode offer it; models use it when asked to.
    settings.subagents = true;
    settings.judge = judge.is_some();
    let identity = identity::load_or_make(&home::folder()?).await?;
    settings.person = Some(identity.person);
    settings.agent = Some(identity.agent);
    if let Some(window_tokens) = config::context_window()? {
        settings.compaction.window_tokens = window_tokens;
    }
    let root = workspace.root().to_owned();
    let servers = Servers::start(&home::folder()?, &root).await?;
    let scope = store
        .local_scope(&root, random_uuid(), random_uuid())
        .await?;
    let wiring = Wiring {
        provider,
        log: store.log(),
        blobs: store.blobs(),
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        workspace,
        tools: servers.sources(),
    };
    Ok(Wired {
        server: Server::new(wiring, settings, scope, answering),
        servers,
        endpoint,
        root,
        files: instructions.map(|loaded| loaded.files).unwrap_or_default(),
    })
}

async fn run(args: Args) -> anyhow::Result<ExitCode> {
    // Ask in the terminal when there is one; otherwise no one is there to answer.
    let asks = std::io::stdin().is_terminal();
    let answering = if asks {
        Answering::Clients
    } else {
        Answering::NoOne
    };
    let wired = wire(&args, answering).await?;
    let ran = session(&args, &wired, asks).await;
    wired.server.shutdown().await;
    wired.servers.close().await;
    ran
}

/// Serves the session API: to the program that started glave until its input ends, or over
/// HTTP until Ctrl-C.
async fn serve(args: Args) -> anyhow::Result<ExitCode> {
    let wired = wire(&args, Answering::Clients).await?;
    let served = if let Some(address) = args.http {
        serve_http(&args, wired.server, address).await
    } else {
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        glave_session::rpc::serve(wired.server, input, tokio::io::stdout())
            .await
            .map_err(anyhow::Error::from)
    };
    wired.servers.close().await;
    served?;
    Ok(ExitCode::SUCCESS)
}

/// Serves the session API over HTTP on `address`, each client signed in by OIDC.
async fn serve_http(args: &Args, server: Glave, address: SocketAddr) -> anyhow::Result<()> {
    let trust = Trust {
        issuers: args.issuers.clone(),
        audience: args.audience.clone().unwrap_or_default(),
    };
    let signin = SignIn::new(trust, Box::new(SystemClock))?;
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("could not listen on {address}"))?;
    eprintln!(
        "glave: serving the session API on http://{}",
        listener.local_addr()?
    );
    let stop = async {
        // Without Ctrl-C, the server runs until it is stopped some other way.
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    glave_session::http::serve(server, signin, listener, stop).await?;
    Ok(())
}

/// Picks the thread, runs the turn or the session, and exports the events when asked.
async fn session(args: &Args, wired: &Wired, asks: bool) -> anyhow::Result<ExitCode> {
    let Wired {
        server,
        servers,
        endpoint,
        root,
        files,
    } = wired;
    let (thread, note) = pick_thread(server, args.resume).await?;
    let input = Input::start();
    let client = Client {
        server,
        thread,
        input: &input,
        endpoint,
        asks,
        servers,
    };
    let mut printer = Printer::new(&endpoint.model, protections(args));
    let code = if args.prompt.is_empty() {
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
            .request(Request::ThreadList { limit: Some(1) })
            .await?
        {
            Reply::Threads { threads } => threads.first().map(|thread| thread.id),
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
    match server.request(Request::ThreadStart {}).await? {
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
        if !line.is_empty() {
            code = client.turn(line, printer).await;
        }
    }
    code
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
fn protections(args: &Args) -> String {
    let approval = format!("approval: {}", args.approval.unwrap_or_default().name());
    match args.sandbox {
        Some(level) => {
            let network = if args.network { " + network" } else { "" };
            format!("{approval} · sandbox: {}{network}", level.name())
        }
        None => approval,
    }
}
