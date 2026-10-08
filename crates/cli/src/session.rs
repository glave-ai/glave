//! Wiring the harness together and running one turn or an interactive session.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use glave_event::Outcome;
use glave_harness::{
    AGENTS_MD, Approver, Cancel, Harness, HarnessError, NoOneToAsk, OsEntropy, Parts, Sandbox,
    Settings, SystemClock, Thread, Workspace, agents_md,
};
use glave_ports::EventLog;
use glave_provider_openai::OpenAiCompatible;
use glave_store_sqlite::{SqliteBlobs, SqliteLog, Store};

use crate::args::{self, Action, Args, USAGE};
use crate::config::{self, Endpoint};
use crate::home;
use crate::input::{CtrlC, Input};
use crate::system::random_uuid;
use crate::terminal::{AskInTerminal, Printer};

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
    let mut workspace = Workspace::open(std::env::current_dir()?, std::env::vars_os()).await?;
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
    let store = Store::open(database).await?;
    let mut settings = Settings::new(endpoint.model.clone(), endpoint.provider.provider.clone());
    let personal = home::folder()?.join(AGENTS_MD);
    let instructions = agents_md(workspace.root(), Some(&personal)).await?;
    settings.instructions = instructions.as_ref().map(|loaded| loaded.text.clone());
    settings.approval = args.approval.unwrap_or_default();
    if let Some(window_tokens) = config::context_window()? {
        settings.compaction.window_tokens = window_tokens;
    }
    let wiring = Wiring {
        provider,
        store,
        workspace,
        settings,
        instructions: instructions.map(|loaded| loaded.files).unwrap_or_default(),
    };
    let input = Input::start();
    // Ask in the terminal when there is one; otherwise no one is there to answer.
    if std::io::stdin().is_terminal() {
        let approver = AskInTerminal::new(input.clone());
        session(&args, &endpoint, wiring, approver, input).await
    } else {
        session(&args, &endpoint, wiring, NoOneToAsk, input).await
    }
}

/// What a session is built from.
struct Wiring {
    provider: OpenAiCompatible,
    store: Store,
    workspace: Workspace,
    settings: Settings,
    /// The instruction files the model is given, for the banner.
    instructions: Vec<PathBuf>,
}

async fn session<A: Approver>(
    args: &Args,
    endpoint: &Endpoint,
    wiring: Wiring,
    approver: A,
    input: Input,
) -> anyhow::Result<ExitCode> {
    let root = wiring.workspace.root().to_owned();
    let instruction_files = wiring.instructions;
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
    let mut printer = Printer::new(&endpoint.model, protections(args));
    let code = if args.prompt.is_empty() {
        let folder = root.display().to_string();
        let note = match named(&instruction_files) {
            Some(files) => format!("{note} · following {files}"),
            None => note.to_owned(),
        };
        printer.banner(&endpoint.model, &endpoint.provider.provider, &folder, &note);
        converse(&harness, &mut thread, &input, &mut printer, endpoint.local).await
    } else {
        turn(
            &harness,
            &mut thread,
            &args.prompt,
            &mut printer,
            endpoint.local,
        )
        .await
    };
    if let Some(path) = &args.events {
        export(harness.log(), &thread, path).await?;
    }
    Ok(code)
}

/// The interactive session: each line typed is a turn, until `/exit`, the end of input, or
/// Ctrl-C at the prompt. Ctrl-C during a turn or `/compact` cancels just that.
async fn converse<A: Approver>(
    harness: &Glave<A>,
    thread: &mut Thread,
    input: &Input,
    printer: &mut Printer,
    local: bool,
) -> ExitCode {
    let mut code = ExitCode::SUCCESS;
    loop {
        printer.ask();
        let interrupt = Cancel::new();
        let typed = {
            let _ctrl_c = CtrlC::cancels(&interrupt);
            interrupt.run(input.line()).await
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
            let cancel = Cancel::new();
            let _ctrl_c = CtrlC::cancels(&cancel);
            match harness
                .compact(thread, &cancel, &mut |live| printer.live(live))
                .await
            {
                Ok(_) => {}
                Err(HarnessError::Cancelled) => {
                    printer.error("compaction cancelled; the thread is as it was");
                }
                Err(error) => printer.error(&error.to_string()),
            }
            continue;
        }
        if !line.is_empty() {
            code = turn(harness, thread, line, printer, local).await;
        }
    }
    code
}

/// Runs one turn, which Ctrl-C cancels, and prints its receipt.
async fn turn<A: Approver>(
    harness: &Glave<A>,
    thread: &mut Thread,
    prompt: &str,
    printer: &mut Printer,
    local: bool,
) -> ExitCode {
    let cancel = Cancel::new();
    let _ctrl_c = CtrlC::cancels(&cancel);
    let result = harness
        .run_turn(thread, prompt, &cancel, &mut |live| printer.live(live))
        .await;
    match result {
        Ok(summary) => {
            printer.receipt(&summary, local);
            match summary.outcome {
                Outcome::Completed => ExitCode::SUCCESS,
                // As shells report a program stopped by Ctrl-C.
                Outcome::Cancelled => ExitCode::from(130),
                _ => ExitCode::FAILURE,
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
