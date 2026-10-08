//! Wiring the harness together and running one turn or an interactive session.

use std::path::Path;
use std::process::ExitCode;

use glave_event::Outcome;
use glave_harness::{AllowAll, Approver, Harness, Parts, Settings, Thread, Workspace};
use glave_ports::{EventLog, MemoryBlobs, MemoryLog, OrgId, Scope, SpaceId};
use glave_provider_openai::OpenAiCompatible;

use crate::args::{self, Args, USAGE};
use crate::config::{self, Endpoint};
use crate::system::{OsEntropy, SystemClock, random_uuid};
use crate::terminal::{AskInTerminal, Printer, read_line};

type Glave<A> = Harness<OpenAiCompatible, MemoryLog, MemoryBlobs, A>;

pub fn main() -> ExitCode {
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("glave: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.version {
        println!("glave {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
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
    let workspace = Workspace::open(std::env::current_dir()?).await?;
    let settings = Settings::new(endpoint.model.clone(), endpoint.provider.provider.clone());
    if args.yes {
        session(&args, &endpoint, provider, workspace, settings, AllowAll).await
    } else {
        session(
            &args,
            &endpoint,
            provider,
            workspace,
            settings,
            AskInTerminal::new(),
        )
        .await
    }
}

async fn session<A: Approver>(
    args: &Args,
    endpoint: &Endpoint,
    provider: OpenAiCompatible,
    workspace: Workspace,
    settings: Settings,
    approver: A,
) -> anyhow::Result<ExitCode> {
    let folder = workspace.root().display().to_string();
    let parts = Parts {
        provider,
        log: MemoryLog::new(),
        blobs: MemoryBlobs::new(),
        approver,
        clock: Box::new(SystemClock),
        entropy: Box::new(OsEntropy),
        workspace,
    };
    let harness: Glave<A> = Harness::new(parts, settings);
    let scope = Scope {
        org: OrgId::from_uuid(random_uuid()),
        space: SpaceId::from_uuid(random_uuid()),
    };
    let mut thread = harness.start_thread(scope).await?;
    let mut printer = Printer::new();
    let mut code = ExitCode::SUCCESS;
    if args.prompt.is_empty() {
        printer.banner(&endpoint.model, &endpoint.provider.provider, &folder);
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
async fn export(log: &MemoryLog, thread: &Thread, path: &Path) -> anyhow::Result<()> {
    let events = log.read(&thread.scope, thread.id, None, usize::MAX).await?;
    let mut lines = String::new();
    for event in &events {
        lines.push_str(&event.to_json()?);
        lines.push('\n');
    }
    tokio::fs::write(path, lines).await?;
    Ok(())
}
