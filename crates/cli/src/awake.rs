//! What keeps a person's own computer awake while glave works on it (`specs/one-server.md`,
//! owner decision 10): a program the system has for it, started through glave-process when
//! work starts and stopped when the last work ends. On macOS `caffeinate -i`, a power
//! assertion against idle sleep; on Linux `systemd-inhibit --what=idle:sleep`, where systemd
//! is; elsewhere nothing, which Settings says. Each waits on glave itself, so neither outlives
//! a glave that ends without stopping it. The screen may still sleep, and closing the lid still
//! sleeps. The person turns it off in Settings, a choice the session keeps in their settings
//! ledger.
//!
//! Only the person's server keeps it, and only on their own computer: never where people reach
//! glave from elsewhere (`[serve]`'s origin or domain) or sign in through an identity provider
//! (`--issuer`). That glave is a server, kept running by what installed it, which does not
//! sleep. Those are the server's own settings, so the choice follows its configuration rather
//! than guessing at the machine.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use glave_config::policy::Choices;
use glave_ports::BoxFuture;
use glave_process::{Child, ProcessError, Processes, Program};
use glave_session::{KeepAwake, Unawake};

use crate::args::Args;

/// What holds the computer awake while it runs: a program, by its absolute path, and its
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Holder {
    program: PathBuf,
    args: Vec<String>,
}

/// Keeps the computer awake by running its [`Holder`] while held.
struct Inhibitor {
    processes: Processes,
    /// None where the system has nothing to keep the computer awake with.
    holder: Option<Holder>,
}

/// What keeps this computer awake while the person's server works on it, when `args` serve as
/// the person's server on their own computer (`keeps`: `choices` give it no public origin).
pub(crate) fn for_server(
    args: &Args,
    choices: &Choices,
    processes: &Processes,
) -> Option<Arc<dyn KeepAwake>> {
    keeps(args, choices.origin.is_some()).then(|| {
        Arc::new(Inhibitor {
            processes: processes.clone(),
            holder: holder(std::process::id()),
        }) as Arc<dyn KeepAwake>
    })
}

/// Whether `args` serve as the person's server on their own computer: not `reached` from
/// elsewhere, nor signed in to through an identity provider.
fn keeps(args: &Args, reached: bool) -> bool {
    args.person_s_server() && !args.reached_from_elsewhere(reached)
}

/// Starts `holder` in a process group of its own, recorded while it runs.
async fn start(processes: &Processes, holder: &Holder) -> Result<Child, ProcessError> {
    let mut program = Program::new(&holder.program);
    program
        .args(&holder.args)
        .no_input()
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    processes.spawn(&mut program).await
}

impl KeepAwake for Inhibitor {
    fn hold(&self) -> BoxFuture<'_, Option<Box<dyn Send>>> {
        Box::pin(async {
            let holder = self.holder.as_ref()?;
            match start(&self.processes, holder).await {
                Ok(child) => {
                    tracing::debug!("keeping this computer awake while glave works");
                    Some(Box::new(child) as Box<dyn Send>)
                }
                Err(error) => {
                    tracing::warn!(%error, "could not keep this computer awake while glave works");
                    None
                }
            }
        })
    }

    fn cannot(&self) -> Option<Unawake> {
        self.holder.is_none().then_some(CANNOT)
    }
}

/// macOS: `caffeinate -i`, which holds a power assertion against idle sleep until glave, whose
/// process is `glave`, ends (`-w`).
#[cfg(target_os = "macos")]
fn holder(glave: u32) -> Option<Holder> {
    let program = found(&["/usr/bin/caffeinate"])?;
    let args = ["-i", "-w", &glave.to_string()].map(String::from).to_vec();
    Some(Holder { program, args })
}

/// Linux: `systemd-inhibit`, which holds logind's inhibitor against idle and sleep for as long
/// as what it runs runs: `tail`, which ends once glave, whose process is `glave`, does.
#[cfg(target_os = "linux")]
fn holder(glave: u32) -> Option<Holder> {
    let program = found(&["/usr/bin/systemd-inhibit", "/bin/systemd-inhibit"])?;
    let tail = found(&["/usr/bin/tail", "/bin/tail"])?;
    let args = [
        "--what=idle:sleep".to_owned(),
        "--who=glave".to_owned(),
        "--why=glave is working".to_owned(),
        tail.display().to_string(),
        format!("--pid={glave}"),
        "-f".to_owned(),
        "/dev/null".to_owned(),
    ];
    Some(Holder {
        program,
        args: args.to_vec(),
    })
}

/// Elsewhere glave has nothing to keep the computer awake with.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn holder(_glave: u32) -> Option<Holder> {
    None
}

/// Why glave cannot keep this computer awake, where it has nothing to do it with.
#[cfg(target_os = "linux")]
const CANNOT: Unawake = Unawake::NoSystemd;
#[cfg(not(target_os = "linux"))]
const CANNOT: Unawake = Unawake::Nothing;

/// The first of `paths` that is a file: programs by absolute path, so none on `PATH` can stand
/// in for them.
fn found(paths: &[&str]) -> Option<PathBuf> {
    paths
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use glave_ports::block_on;

    use super::*;

    fn args(words: &[&str]) -> Args {
        crate::args::parse(words.iter().map(|word| (*word).to_owned())).unwrap()
    }

    #[test]
    fn only_the_person_s_server_on_their_own_computer_keeps_it_awake() {
        let served = args(&["serve"]);
        assert!(keeps(&served, false));
        // Reached from elsewhere, or signed in to through an identity provider: a server.
        assert!(!keeps(&served, true));
        let signed = args(&[
            "serve",
            "--http",
            "127.0.0.1:0",
            "--issuer",
            "https://id.example.com",
            "--audience",
            "glave",
        ]);
        assert!(!keeps(&signed, false));
        // A program's own server over stdio, and a turn run here, keep nothing.
        assert!(!keeps(&args(&["serve", "--stdio"]), false));
        assert!(!keeps(&args(&["fix", "the", "build"]), false));
    }

    #[test]
    fn the_holder_waits_on_glave_itself() {
        let Some(found) = holder(4_242) else {
            return; // Nothing here keeps a computer awake.
        };
        assert!(found.program.is_absolute());
        if cfg!(target_os = "macos") {
            assert_eq!(found.args, ["-i", "-w", "4242"]);
        } else {
            assert!(found.args.contains(&"--what=idle:sleep".to_owned()));
            assert!(found.args.contains(&"--pid=4242".to_owned()));
        }
    }

    #[test]
    fn the_real_holder_runs_while_held_and_stops_when_let_go() {
        block_on!(async {
            let Some(holder) = holder(std::process::id()) else {
                return; // Nothing here keeps a computer awake.
            };
            let mut child = start(&Processes::unrecorded(), &holder).await.unwrap();
            if cfg!(target_os = "macos") {
                // caffeinate waits on this test's own process, which runs.
                assert!(child.try_wait().unwrap().is_none());
            }
            child.stop().await;
            assert!(child.try_wait().unwrap().is_some());
        });
    }
}
