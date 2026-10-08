//! Wiring the harness together and running one turn or an interactive session.

use std::path::Path;
use std::process::ExitCode;

use glave_event::Outcome;
use glave_harness::{AllowAll, Approver, Harness, Parts, Settings, Thread, Workspace};
use glave_ports::EventLog;
use glave_provider_openai::OpenAiCompatible;
use glave_store_sqlite::{SqliteBlobs, SqliteLog, Store};

use crate::args::{self, Action, Args, USAGE};
use crate::config::{self, Endpoint};
use crate::home;
use crate::system::{OsEntropy, SystemClock, random_uuid};
use crate::terminal::{AskInTerminal, Printer, read_line};

type Glave<A> = Harness<OpenAiCompatible, SqliteLog, SqliteBlobs, A>;

pub fn main() -> ExitCode {
    let args = match args::parse(std::env::args().skip(1)) {
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
        Action::Run => {}
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
    match runtime.block_on(run(args)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> anyhow::Result<ExitCode> {
    let endpoint = config::endpoint(args.model.clone())?;
    let provider = OpenAiCompatible::new(endpoint.provider.clone())?;
    let workspace = Workspace::open(std::env::current_dir()?, std::env::vars_os()).await?;
    let store = Store::open(home::database().await?).await?;
    let settings = Settings::new(endpoint.model.clone(), endpoint.provider.provider.clone());
    let wiring = Wiring {
        provider,
        store,
        workspace,
        settings,
    };
    if args.yes {
        session(&args, &endpoint, wiring, AllowAll).await
    } else {
        session(&args, &endpoint, wiring, AskInTerminal::new()).await
    }
}

/// What a session is built from.
struct Wiring {
    provider: OpenAiCompatible,
    store: Store,
    workspace: Workspace,
    settings: Settings,
}

async fn session<A: Approver>(
    args: &Args,
    endpoint: &Endpoint,
    wiring: Wiring,
    approver: A,
) -> anyhow::Result<ExitCode> {
    let root = wiring.workspace.root().to_owned();
    let scope = wiring
        .store
        .local_scope(&root, random_uuid(), random_uuid())
        .await?;
    let parts = Parts {
        provider: wiring.provider,
        log: wiring.store.log(),
        blobs: wiring.store.blobs(),
        approver,
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        workspace: wiring.workspace,
    };
    let harness: Glave<A> = Harness::new(parts, wiring.settings);
    let latest = if args.resume {
        harness.log().threads(&scope, 1).await?.first().copied()
    } else {
        None
    };
    let (mut thread, note) = match latest {
        Some(id) => (
            harness.resume_thread(scope, id).await?,
            "continuing your last thread",
        ),
        None if args.resume => (
            harness.start_thread(scope).await?,
            "no earlier thread here; new thread",
        ),
        None => (harness.start_thread(scope).await?, "new thread"),
    };
    let mut printer = Printer::new(&endpoint.model);
    let mut code = ExitCode::SUCCESS;
    if args.prompt.is_empty() {
        let folder = root.display().to_string();
        printer.banner(&endpoint.model, &endpoint.provider.provider, &folder, note);
        loop {
            printer.ask();
            let Some(line) = read_line().await else { break };
            let line = line.trim();
            if line == "/exit" {
                break;
            }
            if !line.is_empty() {
                code = turn(&harness, &mut thread, line, &mut printer, endpoint.local).await;
            }
        }
    } else {
        code = turn(
            &harness,
            &mut thread,
            &args.prompt,
            &mut printer,
            endpoint.local,
        )
        .await;
    }
    if let Some(path) = &args.events {
        export(harness.log(), &thread, path).await?;
    }
    Ok(code)
}

async fn turn<A: Approver>(
    harness: &Glave<A>,
    thread: &mut Thread,
    prompt: &str,
    printer: &mut Printer,
    local: bool,
) -> ExitCode {
    let result = harness
        .run_turn(thread, prompt, &mut |live| printer.live(live))
        .await;
    match result {
        Ok(summary) => {
            printer.receipt(&summary, local);
            if summary.outcome == Outcome::Completed {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            printer.error(&error.to_string());
            ExitCode::FAILURE
        }
    }
}

/// Writes the thread's events as JSON Lines, one event per line, exactly as stored.
async fn export<L: EventLog>(log: &L, thread: &Thread, path: &Path) -> anyhow::Result<()> {
    let events = log.read(&thread.scope, thread.id, None, usize::MAX).await?;
    let mut lines = String::new();
    for event in &events {
        lines.push_str(&event.to_json()?);
        lines.push('\n');
    }
    tokio::fs::write(path, lines).await?;
    Ok(())
}
