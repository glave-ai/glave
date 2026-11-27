//! `glave computer serve`: this machine as a computer for a harness elsewhere, over the
//! computer protocol (`specs/computers.md`) on stdio or on a Unix socket; and `glave computer bridge`, which carries one connection between its standard
//! input and output and that socket (how a computer in a Pod is reached, through an exec).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use glave_computer::{
    Idle, Keeper, Largest, LocalComputers, Tokens, bridge, listen_socket, serve, serve_socket,
};
use glave_ports::computer::Computers;
use glave_ports::{Clock as _, SystemClock};
use glave_process::Processes;
use tokio::io::BufReader;

use crate::home;
use crate::session::{parsed, runtime};

mod fence;

pub(crate) const USAGE: &str = "\
glave computer: this machine as a computer for a harness elsewhere

Usage:
  glave computer serve --stdio       serve the computer protocol (JSON-RPC 2.0, one message
                                     per line) to the program that started glave
  glave computer serve --socket <path> <folder> [--idle <minutes>] [--keep-disk]
                       [--fence <address:port>]
                                     serve it on a Unix socket at <path>, open to this user
                                     alone, to clients that first send the token in <folder>
                                     (its file: the secret, then how many seconds it opens
                                     connections, counted on this machine's clock from when
                                     glave first reads it; read again for each connection, so
                                     it can be renewed), which no command in the sandbox can
                                     read; how a computer in a Pod is reached. With
                                     --idle, it ends once no request has run and no command
                                     has run in the background for that many minutes, as a
                                     person's computer in the cloud does. With --keep-disk,
                                     whenever more than 80% of the disk $HOME is on is used
                                     and no request or command runs, it frees what can be
                                     rebuilt and went untouched for an hour (build outputs
                                     git ignores in its worktrees, caches, then the Trash),
                                     recorded in $GLAVE_HOME/disk.log. With --fence, it serves
                                     only once <address:port> (its node's kubelet) is no longer
                                     reached, so its network policy holds, and stops after 20
                                     seconds if it still is
  glave computer bridge <path>       carry one connection between standard input and output
                                     and the computer serving on the socket at <path>,
                                     waiting up to 30 seconds for it to be there; it ends
                                     when the computer closes the connection

Copies of repositories are kept in $GLAVE_HOME/workspaces. Commands get glave's environment
without anything named like a key, token, secret or password.";

/// What `glave computer` was asked to do.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    /// Serve this machine as a computer.
    Serve(Transport),
    /// Carry one connection to the computer serving on the socket at the path.
    Bridge(PathBuf),
}

/// How to serve.
#[derive(Debug, PartialEq, Eq)]
enum Transport {
    Stdio,
    /// The socket's path, the folder of the token, and how a person's computer runs.
    Socket(PathBuf, PathBuf, Person),
}

/// How a computer on a socket runs as a person's computer in the cloud: the minutes idle
/// after which it ends, whether it keeps its disk from filling, and the address inside the
/// cluster it must no longer reach before it serves.
#[derive(Debug, Default, PartialEq, Eq)]
struct Person {
    idle: Option<u32>,
    keep_disk: bool,
    fence: Option<SocketAddr>,
}

/// What follows `--socket <path> <folder>`: `--idle <minutes>`, `--keep-disk` and
/// `--fence <address:port>`, each at most once, in any order.
fn person(mut rest: &[String]) -> Result<Person, String> {
    let mut person = Person::default();
    loop {
        rest = match rest {
            [] => return Ok(person),
            [flag, more @ ..] if flag == "--keep-disk" && !person.keep_disk => {
                person.keep_disk = true;
                more
            }
            [flag, at, more @ ..] if flag == "--fence" && person.fence.is_none() => {
                person.fence = Some(fence::address(at)?);
                more
            }
            [flag] if flag == "--fence" => return Err("--fence needs an address:port".into()),
            [flag] if flag == "--idle" => return Err("--idle needs a number of minutes".into()),
            [flag, minutes, more @ ..] if flag == "--idle" && person.idle.is_none() => {
                match minutes.parse::<u32>() {
                    Ok(minutes) if minutes > 0 => person.idle = Some(minutes),
                    _ => return Err(format!("--idle {minutes:?} is not a number of minutes")),
                }
                more
            }
            [flag, ..] if ["--idle", "--keep-disk", "--fence"].contains(&flag.as_str()) => {
                return Err(format!("{flag} is given more than once"));
            }
            [other, ..] => return Err(format!("{other:?} is not an option of serve --socket")),
        };
    }
}

fn parse(arguments: &[String]) -> Result<Asked, String> {
    let serving = |transport| Ok(Asked::Serve(transport));
    match arguments {
        [serve, stdio] if serve == "serve" && stdio == "--stdio" => serving(Transport::Stdio),
        [serve, socket, path, folder, rest @ ..] if serve == "serve" && socket == "--socket" => {
            serving(Transport::Socket(
                PathBuf::from(path),
                PathBuf::from(folder),
                person(rest)?,
            ))
        }
        [bridge, path] if bridge == "bridge" => Ok(Asked::Bridge(PathBuf::from(path))),
        _ => Err(
            "expected: glave computer serve --stdio, or --socket <path> <folder> [--idle <minutes>] [--keep-disk]; \
             or glave computer bridge <path>"
                .into(),
        ),
    }
}

pub(crate) fn main(arguments: &[String]) -> ExitCode {
    let asked = match parsed(arguments, USAGE, parse) {
        Ok(asked) => asked,
        Err(code) => return code,
    };
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(code) => return code,
    };
    let ran = match asked {
        Asked::Bridge(path) => {
            let carried = runtime.block_on(bridge(&path, tokio::io::stdin(), tokio::io::stdout()));
            // Standard input is read on a thread of its own, which a read still waiting would
            // otherwise hold the runtime open on.
            runtime.shutdown_background();
            carried.with_context(|| format!("bridging to {}", path.display()))
        }
        Asked::Serve(transport) => runtime.block_on(run(transport)),
    };
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("glave: {error:#}");
            // A fence that never held ends glave with a code of its own, which the server reads
            // to tell the person in plain words, keeping why for its log.
            if error.downcast_ref::<fence::Unfenced>().is_some() {
                ExitCode::from(glave_computer::UNFENCED)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

async fn run(transport: Transport) -> anyhow::Result<()> {
    let home = home::folder()?;
    // `glave computer serve` writes no log of its own: it says things plainly.
    let processes = home::reaped(crate::said::Said::Plainly).await?;
    let largest = Largest::default();
    let keeping = Keeping {
        processes: processes.clone(),
        glave: home.clone(),
        largest: largest.clone(),
    };
    // The token a computer serves with is its own and no command's to read: with it, a
    // sandboxed command with the network could connect and open a terminal outside every
    // sandbox (#1502). Nor is the socket, which such a command may otherwise reach. Hidden here
    // whatever a command's sandbox names as glave's folder.
    let hidden: Vec<PathBuf> = match &transport {
        Transport::Socket(path, folder, _) => vec![folder.clone(), path.clone()],
        Transport::Stdio => Vec::new(),
    };
    let computers = crate::helpers::with_helper(
        LocalComputers::new(home.join("workspaces"), std::env::vars_os())
            .with_worktrees(home.join(crate::worktrees::FOLDER))
            .with_scratch(home.join(crate::worktrees::SCRATCH))
            .with_projects(home::projects()?)
            .with_hidden(hidden)
            .with_largest(largest)
            .with_processes(processes),
    );
    let computers = Arc::new(crate::helpers::with_glave_on_path(computers, &home).await?);
    match transport {
        Transport::Stdio => {
            let input = BufReader::new(tokio::io::stdin());
            serve(computers, input, tokio::io::stdout(), None)
                .await
                .context("serving on stdio")
        }
        Transport::Socket(path, folder, person) => {
            let busy = {
                let computers = Arc::clone(&computers);
                move || computers.busy()
            };
            on_socket(computers, busy, (&path, &folder), person, keeping).await
        }
    }
}

/// Serves `computers` on the socket at the first path, with the token in the folder at the
/// second, as a person's computer runs as `person` says; `busy` says whether anything runs
/// there in the background.
async fn on_socket<C: Computers + 'static>(
    computers: Arc<C>,
    busy: impl Fn() -> bool + Clone + Send + Sync + 'static,
    (path, folder): (&Path, &Path),
    person: Person,
    keeping: Keeping,
) -> anyhow::Result<()> {
    let token = tokens(folder).await?;
    if let Some(inside) = person.fence {
        let fencing = tokio::time::Instant::now();
        let held = fence::held(inside, fence::HOLD_WITHIN).await?;
        // How long the cluster's plugin took, and the slowest answer before it, in the computer's
        // log, so each cluster's is known and the knock's wait can be checked against it.
        let slowest = held.slowest.map_or_else(
            || "none".to_owned(),
            |took| format!("{} ms", took.as_millis()),
        );
        eprintln!(
            "glave: fenced: {inside} unanswered after {} knocks, {} ms; slowest answer before: \
             {slowest}",
            held.knocks,
            fencing.elapsed().as_millis()
        );
    }
    let listener =
        listen_socket(path).with_context(|| format!("could not listen on {}", path.display()))?;
    let idle = person.idle.map(|minutes| Idle {
        after: Duration::from_mins(u64::from(minutes)),
        busy: Box::new(busy.clone()),
    });
    let keeper = person.keep_disk.then(|| {
        Keeper::new(
            keeping.glave.clone(),
            keeping.glave.join(crate::worktrees::FOLDER),
            keeping.processes,
            std::env::vars_os().collect(),
            Box::new(busy),
            keeping.largest,
        )
    });
    serve_socket(computers, listener, token, idle, keeper)
        .await
        .with_context(|| format!("serving on {}", path.display()))
}

/// What a person's computer keeps its disk with: the processes git runs as, and glave's
/// folder, which holds the worktrees and the record.
struct Keeping {
    processes: Processes,
    glave: PathBuf,
    largest: Largest,
}

/// The token in `folder`, read again for each connection so the harness can renew it, and
/// read once now, so a missing or malformed one stops glave before it listens.
async fn tokens(folder: &Path) -> anyhow::Result<Tokens> {
    let token = Tokens::file(folder.join("token"));
    token
        .current(SystemTime::from(SystemClock.now()))
        .await
        .with_context(|| format!("could not read {}", folder.join("token").display()))?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn serve_takes_one_transport() {
        assert_eq!(
            parse(&words("serve --stdio")),
            Ok(Asked::Serve(Transport::Stdio))
        );
        assert!(parse(&words("serve --tcp 7070")).is_err());
        assert!(parse(&words("serve --tls 0.0.0.0:7070")).is_err());
        assert!(parse(&words("serve")).is_err());
        assert!(parse(&words("serve --stdio --socket a b")).is_err());
        assert!(parse(&words("run")).is_err());
    }

    #[test]
    fn serve_on_a_socket_takes_an_idle_limit_and_a_bridge_takes_its_path() {
        let socket = || PathBuf::from("/opt/glave/glave.sock");
        let folder = || PathBuf::from("/opt/glave/secret");
        assert_eq!(
            parse(&words(
                "serve --socket /opt/glave/glave.sock /opt/glave/secret"
            )),
            Ok(Asked::Serve(Transport::Socket(
                socket(),
                folder(),
                Person::default()
            )))
        );
        assert_eq!(
            parse(&words(
                "serve --socket /opt/glave/glave.sock /opt/glave/secret --idle 10"
            )),
            Ok(Asked::Serve(Transport::Socket(
                socket(),
                folder(),
                Person {
                    idle: Some(10),
                    keep_disk: false,
                    fence: None
                }
            )))
        );
        assert_eq!(
            parse(&words(
                "serve --socket /opt/glave/glave.sock /opt/glave/secret --keep-disk --idle 10"
            )),
            Ok(Asked::Serve(Transport::Socket(
                socket(),
                folder(),
                Person {
                    idle: Some(10),
                    keep_disk: true,
                    fence: None
                }
            )))
        );
        assert_eq!(
            parse(&words(
                "serve --socket /opt/glave/glave.sock /opt/glave/secret --keep-disk --fence \
                 10.0.0.7:10250"
            )),
            Ok(Asked::Serve(Transport::Socket(
                socket(),
                folder(),
                Person {
                    idle: None,
                    keep_disk: true,
                    fence: Some(SocketAddr::from(([10, 0, 0, 7], 10250)))
                }
            )))
        );
        for wrong in [
            "--idle 0",
            "--idle",
            "--idle 1 --idle 2",
            "--keep-disk --keep-disk",
            "--fence",
            "--fence node:10250",
            "--fence 10.0.0.7:10250 --fence 10.0.0.8:10250",
            "--other",
        ] {
            let line = format!("serve --socket /opt/glave/glave.sock /opt/glave/secret {wrong}");
            assert!(parse(&words(&line)).is_err(), "{wrong}");
        }
        assert!(parse(&words("serve --socket /opt/glave/glave.sock")).is_err());
        assert_eq!(
            parse(&words("bridge /opt/glave/glave.sock")),
            Ok(Asked::Bridge(socket()))
        );
        assert!(parse(&words("fence")).is_err());
        assert!(parse(&words("bridge")).is_err());
        assert!(parse(&words("bridge a b")).is_err());
    }
}
