//! `cargo xtask try-org`: a throwaway glave for trying an organization's screens by hand on
//! this computer, its people sharing it each in a browser of their own (`specs/session.md`,
//! "Doors"). It builds the web app, then serves glave from a folder of its own in the
//! temporary folder, at a port the system gives it, its origin this computer's `localhost` at
//! that port, with a key file there and nothing of the person's own glave: not their
//! `GLAVE_HOME`, `~/.glave`, the keychain, or any `GLAVE_` setting of theirs. Its model is a
//! stand-in on this computer (`model.rs`) that answers every chat and says what each answer
//! cost. glave prints the link to sign in with. On
//! Ctrl-C, a hang-up, the end of the program that started xtask or the server's own end, it
//! stops the server and removes the folder. glave's lifeline stops the server if xtask is
//! killed outright; a folder such a run left is removed by the next, which finds its lock free.

mod model;

use std::fs::{File, TryLockError};
use std::io::IsTerminal as _;
use std::os::unix::process::parent_id;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use glave_process::{Processes, Program};
use tokio::signal::unix::{SignalKind, signal};

/// The folder in the temporary folder that holds each run's own.
const RUNS: &str = "glave-try-org";

/// glave's settings: people who share this computer reach it at its `localhost`, at whichever
/// port it serves.
const SETTINGS: &str = "[serve]\norigin = \"http://localhost\"\n";

/// How often xtask looks whether the program that started it has ended.
const PARENT_EVERY: Duration = Duration::from_secs(1);

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("try-org takes no arguments".into());
    }
    if !std::io::stderr().is_terminal() {
        return Err(
            "try-org is for a person at a terminal, where glave prints the link to sign in with"
                .into(),
        );
    }
    let root = crate::workspace::root()?;
    crate::cmd::run_in(&root.join("web"), "npm", &["run", "build"], &[])?;
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
    let served = lock.and_then(|_held| serve(&root, &folder));
    let removed = std::fs::remove_dir_all(&folder)
        .map_err(|err| format!("could not remove {}: {err}", folder.display()));
    // The folder of runs too, once no other run's is in it.
    let _ = std::fs::remove_dir(&runs);
    served.and(removed)
}

/// Serves glave from a folder in `folder` until Ctrl-C, a hang-up, xtask's parent's end or its
/// own: glave's folder, `home`, with the projects glave makes beside it (`home-projects`).
fn serve(root: &Path, folder: &Path) -> Result<(), String> {
    let home = folder.join("home");
    std::fs::create_dir_all(&home)
        .and_then(|()| std::fs::write(home.join("config.toml"), SETTINGS))
        .map_err(|err| format!("could not write glave's settings: {err}"))?;
    let web = root.join("web/dist/app/browser");
    let failed = |err: &dyn std::fmt::Display| format!("could not run glave: {err}");
    crate::cmd::with_lifeline(|processes: Processes| async move {
        let model = model::start()?;
        let mut command = Program::new("cargo");
        command
            .args([
                "run",
                "--quiet",
                "-p",
                "glave",
                "--",
                "serve",
                "--http",
                "127.0.0.1:0",
            ])
            .arg("--web")
            .arg(&web)
            // Nothing of the person's own glave: none of their settings, model or store.
            .env_clear()
            .envs(
                std::env::vars_os()
                    .filter(|(name, _)| !name.to_string_lossy().starts_with("GLAVE_")),
            )
            .env("GLAVE_HOME", &home)
            .env("GLAVE_BASE_URL", format!("{}/v1", model.url))
            .env("GLAVE_MODEL", model::MODEL)
            .current_dir(root)
            .stdin(Stdio::null());
        let mut child = processes
            .spawn(&mut command)
            .await
            .map_err(|err| failed(&err))?;
        eprintln!(
            "xtask: a throwaway glave for people who share this computer, in {}\n\
             xtask: you, its owner: open the sign-in link glave prints, at 127.0.0.1\n\
             xtask: someone you invite: a private window, opening the invite's link, at the \
             localhost address glave prints\n\
             xtask: its model is a stand-in that answers every message the same, and says each \
             answer cost ${}\n\
             xtask: Ctrl-C stops it and removes it all",
            folder.display(),
            model::COST
        );
        let mut hangup = signal(SignalKind::hangup()).map_err(|err| failed(&err))?;
        let mut terminate = signal(SignalKind::terminate()).map_err(|err| failed(&err))?;
        // The person at the terminal is the limit: until they stop it, or it ends.
        tokio::select! {
            ended = child.wait() => {
                ended.map_err(|err| failed(&err))?;
            }
            _ = tokio::signal::ctrl_c() => {}
            _ = hangup.recv() => {}
            _ = terminate.recv() => {}
            () = orphaned() => {}
        }
        child.stop().await;
        drop(model);
        Ok(())
    })
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
