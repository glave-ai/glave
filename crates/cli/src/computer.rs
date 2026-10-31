//! `glave computer serve`: this machine as a computer for a harness elsewhere, over the
//! computer protocol (`specs/computers.md`) on stdio, on TCP at 127.0.0.1, over TLS at any
//! address (how a worker on Docker is reached), or on a Unix socket; and `glave computer
//! bridge`, which carries one connection between its standard input and output and that socket
//! (how a computer in a Pod is reached, through an exec).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, bail};
use glave_computer::{
    Identity, Idle, Keeper, LocalComputers, MIN_SECRET, Tokens, bridge, listen_socket, serve,
    serve_socket, serve_tcp, serve_tls,
};
use glave_ports::computer::Computers;
use glave_ports::{Clock as _, SystemClock};
use glave_process::Processes;
use tokio::io::{AsyncReadExt as _, BufReader};
use tokio::net::TcpListener;

use crate::home;

pub(crate) const USAGE: &str = "\
glave computer: this machine as a computer for a harness elsewhere

Usage:
  glave computer serve --stdio       serve the computer protocol (JSON-RPC 2.0, one message
                                     per line) to the program that started glave
  glave computer serve --tcp <port>  serve it on 127.0.0.1:<port> to clients that first send
                                     connection/authenticate with $GLAVE_COMPUTER_TOKEN
  glave computer serve --tls <host:port> <folder> [--until-input-ends]
                                     serve it over TLS on any address, with cert.pem, key.pem
                                     and token in <folder> (the token file: the secret, then
                                     how many seconds it opens connections, counted on this
                                     machine's clock from when glave first reads it; read
                                     again for each connection, so it can be renewed); how
                                     a harness reaches a worker on Docker. With
                                     --until-input-ends, it ends when its standard input
                                     does, as a worker ends with the glave that started it
  glave computer serve --socket <path> <folder> [--idle <minutes>] [--keep-disk]
                                     serve it on a Unix socket at <path>, open to this user
                                     alone, to clients that first send the token in <folder>
                                     (as for --tls); how a computer in a Pod is reached. With
                                     --idle, it ends once no request has run and no command
                                     has run in the background for that many minutes, as a
                                     person's computer in the cloud does. With --keep-disk,
                                     whenever more than 80% of the disk $HOME is on is used
                                     and no request or command runs, it frees what can be
                                     rebuilt and went untouched for an hour (build outputs
                                     git ignores in its worktrees, caches, then the Trash),
                                     recorded in $GLAVE_HOME/disk.log
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
    Tcp(SocketAddr),
    /// The address, the folder of the identity and token, and until when it serves.
    Tls(SocketAddr, PathBuf, Until),
    /// The socket's path, the folder of the token, and how a person's computer runs.
    Socket(PathBuf, PathBuf, Person),
}

/// How a computer on a socket runs as a person's computer in the cloud: the minutes idle
/// after which it ends, and whether it keeps its disk from filling.
#[derive(Debug, Default, PartialEq, Eq)]
struct Person {
    idle: Option<u32>,
    keep_disk: bool,
}

/// Until when glave serving over TLS serves.
#[derive(Debug, PartialEq, Eq)]
enum Until {
    /// Until accepting fails, or it is stopped.
    Stopped,
    /// Until its standard input ends, as a worker on Docker ends with the glave that started
    /// it.
    InputEnds,
}

/// What follows `--socket <path> <folder>`: `--idle <minutes>` and `--keep-disk`, each at
/// most once, in either order.
fn person(mut rest: &[String]) -> Result<Person, String> {
    let mut person = Person::default();
    loop {
        rest = match rest {
            [] => return Ok(person),
            [flag, more @ ..] if flag == "--keep-disk" && !person.keep_disk => {
                person.keep_disk = true;
                more
            }
            [flag] if flag == "--idle" => return Err("--idle needs a number of minutes".into()),
            [flag, minutes, more @ ..] if flag == "--idle" && person.idle.is_none() => {
                match minutes.parse::<u32>() {
                    Ok(minutes) if minutes > 0 => person.idle = Some(minutes),
                    _ => return Err(format!("--idle {minutes:?} is not a number of minutes")),
                }
                more
            }
            [flag, ..] if flag == "--idle" || flag == "--keep-disk" => {
                return Err(format!("{flag} is given more than once"));
            }
            [other, ..] => return Err(format!("{other:?} is not an option of serve --socket")),
        };
    }
}

fn address(text: &str, flag: &str) -> Result<SocketAddr, String> {
    match text.parse::<u16>() {
        Ok(port) => Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, port))),
        Err(_) => text
            .parse()
            .map_err(|_| format!("{flag} {text:?} is not a port or host:port")),
    }
}

fn parse(arguments: &[String]) -> Result<Asked, String> {
    let serving = |transport| Ok(Asked::Serve(transport));
    match arguments {
        [serve, stdio] if serve == "serve" && stdio == "--stdio" => serving(Transport::Stdio),
        [serve, tcp, at] if serve == "serve" && tcp == "--tcp" => {
            let address = address(at, "--tcp")?;
            if address.ip().is_loopback() {
                serving(Transport::Tcp(address))
            } else {
                Err(format!(
                    "--tcp {address} is not on this machine; serving computers across machines \
                     needs --tls"
                ))
            }
        }
        [serve, tls, at, folder, rest @ ..] if serve == "serve" && tls == "--tls" => {
            let until = match rest {
                [] => Until::Stopped,
                [flag] if flag == "--until-input-ends" => Until::InputEnds,
                [other, ..] => return Err(format!("{other:?} is not an option of serve --tls")),
            };
            serving(Transport::Tls(
                address(at, "--tls")?,
                PathBuf::from(folder),
                until,
            ))
        }
        [serve, socket, path, folder, rest @ ..] if serve == "serve" && socket == "--socket" => {
            serving(Transport::Socket(
                PathBuf::from(path),
                PathBuf::from(folder),
                person(rest)?,
            ))
        }
        [bridge, path] if bridge == "bridge" => Ok(Asked::Bridge(PathBuf::from(path))),
        _ => Err(
            "expected: glave computer serve --stdio, --tcp <port>, --tls <host:port> <folder> \
             [--until-input-ends], or --socket <path> <folder> [--idle <minutes>] [--keep-disk]; \
             or glave \
             computer bridge <path>"
                .into(),
        ),
    }
}

pub(crate) fn main(arguments: &[String]) -> ExitCode {
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let asked = match parse(arguments) {
        Ok(asked) => asked,
        Err(message) => {
            eprintln!("glave: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("glave: could not start: {error}");
            return ExitCode::FAILURE;
        }
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
            ExitCode::FAILURE
        }
    }
}

async fn read(folder: &Path, name: &str) -> anyhow::Result<String> {
    let path = folder.join(name);
    tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("could not read {}", path.display()))
}

async fn run(transport: Transport) -> anyhow::Result<()> {
    let home = home::folder()?;
    let processes = home::reaped().await?;
    let keeping = Keeping {
        processes: processes.clone(),
        glave: home.clone(),
    };
    // The token a computer on a socket serves with is its own, and no command's to read.
    let hidden: Vec<PathBuf> = match &transport {
        Transport::Socket(_, folder, _) => vec![folder.clone()],
        Transport::Stdio | Transport::Tcp(_) | Transport::Tls(..) => Vec::new(),
    };
    let computers = crate::helpers::with_sandbox_helper(
        LocalComputers::new(home.join("workspaces"), std::env::vars_os())
            .with_worktrees(home.join(crate::worktrees::FOLDER))
            .with_scratch(home.join(crate::worktrees::SCRATCH))
            .with_projects(home::projects()?)
            .with_hidden(hidden)
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
        Transport::Tcp(address) => {
            let Some(token) = std::env::var("GLAVE_COMPUTER_TOKEN")
                .ok()
                .filter(|token| token.len() >= MIN_SECRET)
            else {
                bail!(
                    "glave computer serve --tcp needs GLAVE_COMPUTER_TOKEN, at least \
                     {MIN_SECRET} characters: a listener never runs without one"
                );
            };
            let listener = TcpListener::bind(address)
                .await
                .with_context(|| format!("could not listen on {address}"))?;
            serve_tcp(computers, listener, token)
                .await
                .with_context(|| format!("serving on {address}"))
        }
        Transport::Tls(address, folder, until) => {
            let tls = Identity::acceptor(
                &read(&folder, "cert.pem").await?,
                &read(&folder, "key.pem").await?,
            )?;
            let token = tokens(&folder).await?;
            let listener = TcpListener::bind(address)
                .await
                .with_context(|| format!("could not listen on {address}"))?;
            let serving = serve_tls(computers, listener, tls, token);
            if until == Until::Stopped {
                return serving
                    .await
                    .with_context(|| format!("serving TLS on {address}"));
            }
            tokio::select! {
                served = serving => served.with_context(|| format!("serving TLS on {address}")),
                () = input_ends() => Ok(()),
            }
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

/// Reads standard input to its end, or to an error, discarding what it reads.
async fn input_ends() {
    let mut input = tokio::io::stdin();
    let mut read = [0_u8; 256];
    while matches!(input.read(&mut read).await, Ok(n) if n > 0) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn serve_takes_one_transport_and_only_tls_leaves_this_machine() {
        assert_eq!(
            parse(&words("serve --stdio")),
            Ok(Asked::Serve(Transport::Stdio))
        );
        assert_eq!(
            parse(&words("serve --tcp 7070")),
            Ok(Asked::Serve(Transport::Tcp(
                "127.0.0.1:7070".parse().unwrap()
            )))
        );
        assert!(parse(&words("serve --tcp 0.0.0.0:7070")).is_err());
        assert_eq!(
            parse(&words("serve --tls 0.0.0.0:7070 /opt/glave/tls")),
            Ok(Asked::Serve(Transport::Tls(
                "0.0.0.0:7070".parse().unwrap(),
                PathBuf::from("/opt/glave/tls"),
                Until::Stopped
            )))
        );
        assert_eq!(
            parse(&words(
                "serve --tls 0.0.0.0:7070 /opt/glave/tls --until-input-ends"
            )),
            Ok(Asked::Serve(Transport::Tls(
                "0.0.0.0:7070".parse().unwrap(),
                PathBuf::from("/opt/glave/tls"),
                Until::InputEnds
            )))
        );
        assert!(parse(&words("serve --tls 0.0.0.0:7070 /opt/glave/tls --idle 15")).is_err());
        assert!(parse(&words("serve --tls 0.0.0.0:7070 /opt/glave/tls --other")).is_err());
        assert!(parse(&words("serve --tls 0.0.0.0:7070")).is_err());
        assert!(parse(&words("serve")).is_err());
        assert!(parse(&words("serve --stdio --tcp 1")).is_err());
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
                    keep_disk: false
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
                    keep_disk: true
                }
            )))
        );
        for wrong in [
            "--idle 0",
            "--idle",
            "--idle 1 --idle 2",
            "--keep-disk --keep-disk",
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
        assert!(parse(&words("bridge")).is_err());
        assert!(parse(&words("bridge a b")).is_err());
    }
}
