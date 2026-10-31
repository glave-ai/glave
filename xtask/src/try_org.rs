//! `cargo xtask try-org`: a throwaway glave for trying an organization's screens by hand on
//! this computer, its people sharing it each in a browser of their own (`specs/session.md`,
//! "Doors"). It builds the web app and glave, then serves glave from a folder of its own in the
//! temporary folder, at a port the system gives it, its origin this computer's `localhost` at
//! that port, with a key file there and nothing of the person's own glave: not their
//! `GLAVE_HOME`, `~/.glave`, the keychain, or any `GLAVE_` setting of theirs. Its model is a
//! stand-in on this computer (`model.rs`) that answers every chat and says what each answer
//! cost, and stands in for OpenRouter for an account connected with any key, and it has one
//! service, glave's MCP test server, whose tool the stand-in calls when told to "ask me
//! something", to try what a service asks the person; with `--credits`,
//! it is the server's own OpenRouter, as on Glave Cloud, with no account to connect, and
//! every organization pays for it from credits whoever runs the glave adds (`specs/gateway.md`,
//! "Credits"). Its organization starts with people in it, an Admin and a Member with names, who
//! joined by invites, and an invite waiting (`seed.rs`), so its Admin has something to show at
//! once. Once the server is ready, xtask opens it in the person's browser, signed in, as `glave`
//! alone does, and again each time the person presses Enter in its window, so no link can expire
//! on them. On Ctrl-C, a hang-up, the end of the program that started xtask or the server's own
//! end, it stops the server and removes the folder. glave's lifeline stops the server if xtask
//! is killed outright; a folder such a run left is removed by the next, which finds its lock
//! free.

use std::fs::{File, TryLockError};
use std::io::IsTerminal as _;
use std::os::unix::process::parent_id;
use std::path::{Path, PathBuf};
use std::time::Duration;

use glave_ports::spoken;
use glave_process::{Child, Processes, Program};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

/// The folder in the temporary folder that holds each run's own.
const RUNS: &str = "glave-try-org";

mod model;
mod router;
mod seed;

/// glave's settings: people who share this computer reach it at its `localhost`, at whichever
/// port it serves.
const SETTINGS: &str = "[serve]\norigin = \"http://localhost\"\n";

/// With `--credits`, organizations pay for their models from credits, each limited to a dollar
/// a month, so both refusals are a few chats away.
const CREDITS: &str = "[credits]\nmonthly_limit_usd = 1\n";

/// How often xtask looks whether the program that started it has ended.
const PARENT_EVERY: Duration = Duration::from_secs(1);

/// How long the server may take to say where it is reached once started: as long as glave
/// gives a server it starts for the person, a new build's first start on a busy computer.
const READY_WITHIN: Duration = Duration::from_mins(2);

/// How often xtask looks whether the server is ready.
const READY_EVERY: Duration = Duration::from_millis(200);

/// How long opening the browser may take: glave asks the server for a link, then the system's
/// opener hands a page to the browser.
const OPENS_WITHIN: Duration = Duration::from_secs(30);

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let credits = match args {
        [] => false,
        [flag] if flag == "--credits" => true,
        _ => return Err("try-org takes --credits, or nothing".into()),
    };
    if !std::io::stderr().is_terminal() {
        return Err(
            "try-org is for a person at a terminal, who stops it there and opens it again".into(),
        );
    }
    let root = crate::workspace::root()?;
    crate::cmd::run_in(&root.join("web"), "npm", &["run", "build"], &[])?;
    crate::cmd::run("cargo", &["build", "-p", "glave"])?;
    crate::cmd::run("cargo", &crate::web::TEST_SERVER)?;
    let runs = std::env::temp_dir().join(RUNS);
    sweep(&runs);
    let folder = runs.join(std::process::id().to_string());
    std::fs::create_dir_all(&folder)
        .map_err(|err| format!("could not make {}: {err}", folder.display()))?;
    // Held while this run lasts, so a later one knows the folder is in use.
    let lock = File::create(folder.join("lock"))
        .and_then(|file| {
            file.try_lock()
                .map(|()| file)
                .map_err(std::io::Error::other)
        })
        .map_err(|err| format!("could not hold {}: {err}", folder.display()));
    let served = lock.and_then(|_held| serve(&root, &folder, credits));
    let removed = std::fs::remove_dir_all(&folder)
        .map_err(|err| format!("could not remove {}: {err}", folder.display()));
    // The folder of runs too, once no other run's is in it.
    let _ = std::fs::remove_dir(&runs);
    served.and(removed)
}

/// Serves glave from a folder in `folder` until Ctrl-C, a hang-up, xtask's parent's end or its
/// own: glave's folder, `home`, with the projects glave makes beside it (`home-projects`), its
/// organizations paying from credits when `credits` says.
fn serve(root: &Path, folder: &Path, credits: bool) -> Result<(), String> {
    let home = folder.join("home");
    let settings = if credits {
        format!("{SETTINGS}{CREDITS}")
    } else {
        SETTINGS.to_owned()
    };
    // Asked of cargo before the runtime starts: asking blocks.
    let glave = crate::web::debug_glave()?;
    let service = serde_json::json!({"mcpServers": {model::SERVICE: {
        "command": glave.with_file_name("glave-mcp-test-server").display().to_string(),
    }}});
    std::fs::create_dir_all(&home)
        .and_then(|()| std::fs::write(home.join("config.toml"), settings))
        .and_then(|()| std::fs::write(home.join("mcp.json"), service.to_string()))
        .map_err(|err| format!("could not write glave's settings: {err}"))?;
    let web = root.join("web/dist/app/browser");
    let failed = |err: &dyn std::fmt::Display| format!("could not run glave: {err}");
    crate::cmd::with_lifeline(|processes: Processes| async move {
        let model = model::start()?;
        let router = format!("{}{}", model.url, model::OPENROUTER);
        let mut command = Program::new(&glave);
        command
            .args(["serve", "--http", "127.0.0.1:0"])
            .arg("--web")
            .arg(&web)
            // Nothing of the person's own glave: none of their settings, model or store.
            .env_clear()
            .envs(
                std::env::vars_os()
                    .filter(|(name, _)| !name.to_string_lossy().starts_with("GLAVE_")),
            )
            .env("GLAVE_HOME", &home)
            .env("GLAVE_OPENROUTER_URL", &router)
            .current_dir(root)
            .no_input();
        // After the variables are cleared above.
        if credits {
            // As Glave Cloud: its server's own OpenRouter, with no account of anyone's.
            command
                .env("GLAVE_BASE_URL", &router)
                .env("GLAVE_PROVIDER", "openrouter")
                .env("GLAVE_API_KEY", "stand-in")
                .env("GLAVE_MODEL", model::PRICED_MODEL);
        } else {
            command
                .env("GLAVE_BASE_URL", format!("{}/v1", model.url))
                .env("GLAVE_MODEL", model::MODEL);
        }
        let mut child = processes
            .spawn(&mut command)
            .await
            .map_err(|err| failed(&err))?;
        say_what_it_is(folder, &home, credits);
        let opening = Opening {
            processes: &processes,
            program: glave,
            home: &home,
        };
        let ended = until_stopped(&mut child, &opening).await;
        child.stop().await;
        drop(model);
        ended
    })
}

/// Tells the person at the terminal what they are running, in `folder` with glave's folder at
/// `home`, and how to try it.
fn say_what_it_is(folder: &Path, home: &Path, credits: bool) {
    let router_said = if credits {
        "xtask: its models are the server's own OpenRouter, as on Glave Cloud, with no \
         account to connect: the stand-in serves sample models from sample providers, so \
         choose who serves one in the model picker"
    } else {
        "xtask: to try choosing who serves a model, connect OpenRouter with any key: the \
         stand-in serves sample models from sample providers"
    };
    eprintln!(
        "xtask: a throwaway glave for people who share this computer, in {}\n\
         xtask: you, its owner: it opens in your browser, signed in, once it is ready, with \
         an Admin and a Member who joined by invites, and an invite waiting\n\
         xtask: someone you invite: a private window, opening the invite's link, at the \
         localhost address glave prints\n\
         xtask: its model is a stand-in that answers every message the same, and says each \
         answer cost $0.10\n\
         {router_said}\n\
         xtask: it runs until Ctrl-C in this window, which stops it and removes it all; \
         press Enter here to open it in your browser again",
        folder.display()
    );
    if credits {
        eprintln!(
            "xtask: credits are on, a dollar a month at most until an owner raises it; once \
             it is open, add some and see what is left, as whoever runs it:\n\
             xtask:   GLAVE_HOME={} cargo run -q -p glave -- credits add 1\n\
             xtask:   GLAVE_HOME={} cargo run -q -p glave -- credits",
            home.display(),
            home.display()
        );
    }
}

/// Runs until Ctrl-C, a hang-up, a request to stop, the end of what started xtask or the
/// server's own end, welcoming the person meanwhile: whatever is being done for them stops at
/// once with it.
async fn until_stopped(child: &mut Child, opening: &Opening<'_>) -> Result<(), String> {
    let failed = |err: std::io::Error| format!("could not run glave: {err}");
    let mut interrupt = signal(SignalKind::interrupt()).map_err(failed)?;
    let mut hangup = signal(SignalKind::hangup()).map_err(failed)?;
    let mut terminate = signal(SignalKind::terminate()).map_err(failed)?;
    // The person at the terminal is the limit: until they stop it, or it ends.
    tokio::select! {
        ended = child.wait() => ended.map(drop).map_err(failed),
        _ = interrupt.recv() => Ok(()),
        _ = hangup.recv() => Ok(()),
        _ = terminate.recv() => Ok(()),
        () = orphaned() => Ok(()),
        not_ready = opening.welcome() => Err(not_ready),
    }
}

/// What opens the throwaway glave whose folder is `home` in the person's browser: `glave`
/// alone, `program` the glave that serves it.
struct Opening<'a> {
    processes: &'a Processes,
    program: PathBuf,
    home: &'a Path,
}

impl Opening<'_> {
    /// What the person gets: once glave is ready, its organization's people (`seed.rs`), then
    /// glave in their browser, and again each time they press Enter. It ends only when glave
    /// did not start, saying so.
    async fn welcome(&self) -> String {
        let mut enter = presses();
        if tokio::time::timeout(READY_WITHIN, self.ready())
            .await
            .is_err()
        {
            return format!("glave did not start within {}", spoken(READY_WITHIN));
        }
        if let Err(err) = seed::seed(self.home).await {
            eprintln!("xtask: glave runs, but without its sample people: {err}");
        }
        self.open().await;
        // A press while it started or opened asked for what has happened since.
        while enter.try_recv().is_ok() {}
        while enter.recv().await.is_some() {
            self.open().await;
        }
        // No more input to read: glave runs on until it is stopped.
        std::future::pending().await
    }

    /// Completes once glave says where a browser reaches it. Its listener is bound by then,
    /// and answers once it serves, its socket with it.
    async fn ready(&self) {
        let mut each = tokio::time::interval(READY_EVERY);
        while !self.home.join("serve.url").is_file() {
            each.tick().await;
        }
    }

    /// Opens it at the person's Chats, signed in, as `glave` alone does, and says so, or why
    /// not. What glave says is read, not shown: with no one at its output, it prints no link.
    async fn open(&self) {
        match self.processes.output(&mut self.alone(), OPENS_WITHIN).await {
            Ok(opened) if opened.status.success() => {
                eprintln!("xtask: opened in your browser; press Enter here to open it again");
            }
            Ok(opened) => {
                let said = String::from_utf8_lossy(&opened.stderr);
                let why = said.lines().next_back().unwrap_or("glave said nothing");
                eprintln!("xtask: could not open your browser ({why}); press Enter to try again");
            }
            Err(err) => {
                eprintln!("xtask: could not open your browser ({err}); press Enter to try again");
            }
        }
    }

    /// `glave` alone, for this glave only: none of the person's own settings. Run in the
    /// temporary folder, which is never a project, with no one at its input, it asks nothing
    /// and opens the person's Chats.
    fn alone(&self) -> Program {
        let mut glave = Program::new(&self.program);
        glave
            .env_clear()
            .envs(
                std::env::vars_os()
                    .filter(|(name, _)| !name.to_string_lossy().starts_with("GLAVE_")),
            )
            .env("GLAVE_HOME", self.home)
            .current_dir(std::env::temp_dir())
            .no_input();
        glave
    }
}

/// Each time the person presses Enter in this window, a press while one waits counted once.
/// Read on a thread of its own, as a read of the terminal cannot be cancelled: it ends with
/// xtask, or when the terminal's input does.
fn presses() -> mpsc::Receiver<()> {
    let (press, presses) = mpsc::channel(1);
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            if line.is_err() || press.is_closed() {
                return;
            }
            let _ = press.try_send(());
        }
    });
    presses
}

/// Completes once the program that started xtask has ended, leaving xtask to another parent:
/// looked at every [`PARENT_EVERY`], since macOS tells a child nothing of it.
async fn orphaned() {
    let parent = parent_id();
    let mut each = tokio::time::interval(PARENT_EVERY);
    while parent_id() == parent {
        each.tick().await;
    }
}

/// Removes the folders of runs that ended without removing their own: those whose lock no one
/// holds.
fn sweep(runs: &Path) {
    let Ok(entries) = std::fs::read_dir(runs) else {
        return;
    };
    let left: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    for folder in left {
        let free = File::open(folder.join("lock")).is_ok_and(|file| match file.try_lock() {
            Ok(()) => true,
            Err(TryLockError::WouldBlock | TryLockError::Error(_)) => false,
        });
        if free {
            let _ = std::fs::remove_dir_all(&folder);
        }
    }
}
