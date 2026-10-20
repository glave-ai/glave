//! Memory on this computer (`specs/memory.md`): glave's memory server over the local store,
//! started when the person's `memory` choice is on and the policy allows it, and `glave memory`,
//! which lists, edits, moves and forgets memories from the terminal: through the person's server
//! when it runs (or a terminal starts it) and keeps memory on, otherwise in the store itself.

use std::fmt::Write as _;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context as _;
use glave_event::{MemoryId, MemoryScope, PrincipalId};
use glave_memory::{GlaveMemory, Memories, Personal, Rules, Tidier};
use glave_ports::memories::{Access, Control, Memory, Remembered};
use glave_ports::{ModelProvider, OsEntropy, ProjectId, Scope, SystemClock};
use glave_session::protocol::{ApiError, Reply, Request, code};

use crate::args::Args;
use crate::data::{Data, DataLog};
use crate::identity;
use crate::reach::Reached;
use crate::setup::Setup;
use glave_config::policy::MemorySettings;

pub(crate) const USAGE: &str = "\
glave memory: what glave remembers here, for later threads

Usage:
  glave memory                               list your memories and this project's
  glave memory edit <id> <text>              change a memory's text
  glave memory move <id> personal|project      save it in the other place, forgetting it here
  glave memory forget <id>                   forget a memory: its key is destroyed at once
  glave memory forget --all [--project | --personal]
                                             forget every memory of both, or one, of them

Memory is on when `memory = \"on\"` is in your config.toml (glave policy explain shows it).
In a session, /memory lists them.";

/// Memory as running here: the server, and the projects it keeps.
pub(crate) struct Running {
    pub memory: Arc<GlaveMemory<DataLog>>,
    pub scopes: Vec<Scope>,
}

impl Running {
    /// Memory as the session server and its threads use it.
    pub(crate) fn handle(&self) -> Arc<dyn Memory> {
        Arc::clone(&self.memory) as Arc<dyn Memory>
    }

    /// Tidies this person's and this project's memory in the background with `tidier` (the
    /// gateway's `summary` route): now, when due, and after saves that bring a scope near its
    /// budget.
    pub(crate) fn tidy_with<P: ModelProvider + 'static>(&self, tidier: Tidier<P>) {
        self.memory.tidy_with(tidier);
        self.memory.tidy_soon(&self.scopes);
    }

    /// Housekeeping: retention, erasure that is due, and rotating project keys once keys were
    /// destroyed. Failures are reported, never fatal.
    pub(crate) async fn housekeep(&self) {
        if let Err(error) = self.memory.memories().housekeep(&self.scopes).await {
            eprintln!("glave: memory housekeeping failed: {error}");
        }
    }
}

/// Starts memory for a run with `setup`, when its choices turn it on and the policy allows.
pub(crate) async fn start_for(
    store: &Data,
    setup: &Setup,
    person: PrincipalId,
    scope: Scope,
) -> anyhow::Result<Option<Running>> {
    let settings =
        glave_config::policy::memory_settings(&setup.choices, &setup.loaded.constraints)?;
    start(store, &settings, person, scope).await
}

/// Starts memory for `person`, tidied in `scope` and their own, when `settings` turn it on:
/// each project with the setting `settings` choose, until its project's settings choose another
/// (`project_folders.rs`).
pub(crate) async fn start(
    store: &Data,
    settings: &MemorySettings,
    person: PrincipalId,
    scope: Scope,
) -> anyhow::Result<Option<Running>> {
    if !settings.on {
        return Ok(None);
    }
    let personal = crate::projects::personal(store, person).await?;
    let rules = Rules {
        personal: settings.personal.then_some(Personal {
            scope: personal,
            person,
        }),
        default: settings.project,
        retention_days: settings.retention_days,
        ..Rules::default()
    };
    let memories = Memories::new(
        store.log(),
        store.eraser(),
        rules,
        Box::new(SystemClock),
        Box::new(OsEntropy),
    );
    let memory = GlaveMemory::start(memories)
        .await
        .context("could not start memory")?;
    let running = Running {
        memory: Arc::new(memory),
        scopes: vec![personal, scope],
    };
    running.housekeep().await;
    Ok(Some(running))
}

/// What `glave memory` was asked.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    List,
    Run(Vec<Control>),
}

fn parse(words: &[String]) -> Result<Command, String> {
    let id = |text: &str| {
        text.parse::<MemoryId>()
            .map_err(|_| format!("{text:?} is not a memory id"))
    };
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let control = match words.as_slice() {
        [] | ["list"] => return Ok(Command::List),
        ["edit", memory, text @ ..] if !text.is_empty() => Control::Edit {
            memory: id(memory)?,
            text: text.join(" "),
        },
        ["move", memory, scope] => Control::Move {
            memory: id(memory)?,
            scope: MemoryScope::named(scope).ok_or("move to personal or project")?,
        },
        ["forget", "--all", "--project"] => Control::ForgetAll {
            scope: MemoryScope::Project,
        },
        ["forget", "--all", "--personal"] => Control::ForgetAll {
            scope: MemoryScope::Personal,
        },
        ["forget", "--all"] => {
            return Ok(Command::Run(vec![
                Control::ForgetAll {
                    scope: MemoryScope::Project,
                },
                Control::ForgetAll {
                    scope: MemoryScope::Personal,
                },
            ]));
        }
        ["forget", memory] => Control::Forget {
            memory: id(memory)?,
        },
        [first, ..] => return Err(format!("unexpected {first:?}")),
    };
    Ok(Command::Run(vec![control]))
}

pub(crate) fn main(arguments: &[String]) -> ExitCode {
    if arguments
        .iter()
        .any(|word| word == "--help" || word == "-h")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let command = match parse(arguments) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("glave: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        eprintln!("glave: could not start");
        return ExitCode::FAILURE;
    };
    match runtime.block_on(run(command)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("glave: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> anyhow::Result<()> {
    if let Some(server) = crate::server::connect_or_start().await? {
        match through(&mut Reached::Server(server), &command).await {
            // With memory off, the person still sees and forgets what was kept, here.
            Err(error) if is_off(&error) => {}
            ran => return ran,
        }
    }
    in_the_store(command).await
}

/// Whether the server said memory is off.
fn is_off(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ApiError>()
        .is_some_and(|refused| refused.code == code::MEMORY_OFF)
}

/// `command` through the person's server, in this folder's project.
async fn through(server: &mut Reached, command: &Command) -> anyhow::Result<()> {
    let project = Some(server.project(&std::env::current_dir()?).await?);
    let Command::Run(controls) = command else {
        let memories = changed(
            server
                .request(Request::MemoryList {
                    scope: None,
                    project,
                })
                .await?,
        )?;
        if memories.is_empty() {
            println!("no memories here");
        }
        for memory in &memories {
            println!("{}", line(memory));
        }
        return Ok(());
    };
    for control in controls {
        let verb = verb(control);
        for memory in changed(server.request(asked(control.clone(), project)?).await?)? {
            println!("{verb} {}", line(&memory));
        }
    }
    Ok(())
}

/// The memories a `memory/*` request answered with.
fn changed(reply: Reply) -> anyhow::Result<Vec<Remembered>> {
    match reply {
        Reply::Memories { memories } => Ok(memories),
        _ => anyhow::bail!("glave answered memory/* with something else"),
    }
}

/// The request that makes `control` in `project`.
fn asked(control: Control, project: Option<ProjectId>) -> anyhow::Result<Request> {
    Ok(match control {
        Control::Edit { memory, text } => Request::MemoryEdit {
            memory,
            text,
            project,
        },
        Control::Move { memory, scope } => Request::MemoryMove {
            memory,
            scope,
            project,
        },
        Control::Forget { memory } => Request::MemoryForget {
            memory: Some(memory),
            all: false,
            scope: None,
            project,
        },
        Control::ForgetAll { scope } => Request::MemoryForget {
            memory: None,
            all: true,
            scope: Some(scope),
            project,
        },
        other => anyhow::bail!("glave memory does not make {other:?}"),
    })
}

/// What `control` did, as said.
fn verb(control: &Control) -> &'static str {
    match control {
        Control::Edit { .. } => "edited",
        Control::Move { .. } => "moved",
        _ => "forgot",
    }
}

/// `command` in the store itself, with memory on for it.
async fn in_the_store(command: Command) -> anyhow::Result<()> {
    let folder = std::env::current_dir()?;
    let setup = Setup::load(&Args::default(), &folder).await?;
    let settings =
        glave_config::policy::memory_settings(&setup.choices, &setup.loaded.constraints)?;
    let store = Data::chosen(&setup.choices).await?;
    let person = identity::load_or_make(&store).await?.person;
    let scope = crate::projects::here(&store, &folder, person).await?;
    let on = MemorySettings {
        on: true,
        ..settings
    };
    let Some(running) = start(&store, &on, person, scope).await? else {
        return Ok(());
    };
    if !settings.on {
        eprintln!(
            "glave: memory is off for new threads (memory = \"on\" in config.toml turns it on)"
        );
    }
    let access = Access {
        person: Some(person),
        scope,
    };
    match command {
        Command::List => {
            let memories = running
                .memory
                .control(&access, Control::List { scope: None })
                .await?;
            if memories.is_empty() {
                println!("no memories here");
            }
            for memory in &memories {
                println!("{}", line(memory));
            }
        }
        Command::Run(controls) => {
            for control in controls {
                let verb = verb(&control);
                for memory in running.memory.control(&access, control).await? {
                    println!("{verb} {}", line(&memory));
                }
            }
        }
    }
    running.housekeep().await;
    running.memory.close().await;
    Ok(())
}

/// One memory as listed: id, scope, dates, where it came from, and the text.
pub(crate) fn line(memory: &Remembered) -> String {
    let day = |at: glave_event::At| at.to_string().chars().take(10).collect::<String>();
    let mut about = format!("{}, saved {}", memory.scope.as_str(), day(memory.saved_at));
    if let Some(edited) = memory.edited_at {
        let _ = write!(about, ", edited {}", day(edited));
    }
    if let Some(source) = memory.source {
        let _ = write!(about, ", from thread {}", source.thread);
    }
    if memory.untrusted_read {
        about.push_str(", saved after reading untrusted content");
    }
    format!("{}  ({about})  {}", memory.memory, memory.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &[&str]) -> Vec<String> {
        text.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn the_words_after_memory_pick_a_command() {
        let id = "0192f5c3-0000-7000-8000-000000000001";
        assert_eq!(parse(&words(&[])), Ok(Command::List));
        assert!(matches!(
            parse(&words(&["edit", id, "use", "tabs"])),
            Ok(Command::Run(c)) if matches!(c.as_slice(), [Control::Edit { text, .. }] if text == "use tabs")
        ));
        assert!(matches!(
            parse(&words(&["move", id, "personal"])),
            Ok(Command::Run(c)) if matches!(c.as_slice(), [Control::Move { scope: MemoryScope::Personal, .. }])
        ));
        assert!(matches!(
            parse(&words(&["forget", "--all", "--personal"])),
            Ok(Command::Run(c)) if c.len() == 1
        ));
        assert!(matches!(parse(&words(&["forget", "--all"])), Ok(Command::Run(c)) if c.len() == 2));
        assert!(parse(&words(&["forget", "nope"])).is_err());
        assert!(parse(&words(&["move", id, "elsewhere"])).is_err());
    }
}
