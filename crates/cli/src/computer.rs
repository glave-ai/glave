//! `glave computer serve`: this machine as a computer for a harness elsewhere, over the
//! computer protocol (`specs/computers.md`) on stdio, on TCP at 127.0.0.1, or over TLS at any
//! address (how a worker is reached).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context as _, bail};
use glave_computer::{Identity, LocalComputers, MIN_SECRET, Tokens, serve, serve_tcp, serve_tls};
use tokio::io::BufReader;
use tokio::net::TcpListener;

use crate::home;

pub const USAGE: &str = "\
glave computer: this machine as a computer for a harness elsewhere

Usage:
  glave computer serve --stdio       serve the computer protocol (JSON-RPC 2.0, one message
                                     per line) to the program that started glave
  glave computer serve --tcp <port>  serve it on 127.0.0.1:<port> to clients that first send
                                     connection/authenticate with $GLAVE_COMPUTER_TOKEN
  glave computer serve --tls <host:port> <folder>
                                     serve it over TLS on any address, with cert.pem, key.pem
                                     and token in <folder> (the token file: the secret, then
                                     how many seconds it opens connections, counted on this
                                     machine's clock from when glave first reads it; read
                                     again for each connection, so it can be renewed); how
                                     a harness reaches a worker

Copies of repositories are kept in $GLAVE_HOME/workspaces. Commands get glave's environment
without anything named like a key, token, secret or password.";

/// How to serve.
#[derive(Debug, PartialEq, Eq)]
enum Transport {
    Stdio,
    Tcp(SocketAddr),
    Tls(SocketAddr, PathBuf),
}

fn address(text: &str, flag: &str) -> Result<SocketAddr, String> {
    match text.parse::<u16>() {
        Ok(port) => Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, port))),
        Err(_) => text
            .parse()
            .map_err(|_| format!("{flag} {text:?} is not a port or host:port")),
    }
}

fn parse(arguments: &[String]) -> Result<Transport, String> {
    match arguments {
        [serve, stdio] if serve == "serve" && stdio == "--stdio" => Ok(Transport::Stdio),
        [serve, tcp, at] if serve == "serve" && tcp == "--tcp" => {
            let address = address(at, "--tcp")?;
            if address.ip().is_loopback() {
                Ok(Transport::Tcp(address))
            } else {
                Err(format!(
                    "--tcp {address} is not on this machine; serving computers across machines \
                     needs --tls"
                ))
            }
        }
        [serve, tls, at, folder] if serve == "serve" && tls == "--tls" => {
            Ok(Transport::Tls(address(at, "--tls")?, PathBuf::from(folder)))
        }
        _ => Err(
            "expected: glave computer serve --stdio, --tcp <port>, or --tls <host:port> <folder>"
                .into(),
        ),
    }
}

pub fn main(arguments: &[String]) -> ExitCode {
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let transport = match parse(arguments) {
        Ok(transport) => transport,
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
    match runtime.block_on(run(transport)) {
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
    let computers = Arc::new(
        LocalComputers::new(home.join("workspaces"), std::env::vars_os())
            .with_worktrees(home.join(crate::worktrees::FOLDER)),
    );
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
        Transport::Tls(address, folder) => {
            let tls = Identity::acceptor(
                &read(&folder, "cert.pem").await?,
                &read(&folder, "key.pem").await?,
            )?;
            // Read for each connection, so the harness can renew it; read once now, so a
            // missing or malformed token stops glave before it listens.
            let token = Tokens::file(folder.join("token"));
            token
                .current(SystemTime::now())
                .await
                .with_context(|| format!("could not read {}", folder.join("token").display()))?;
            let listener = TcpListener::bind(address)
                .await
                .with_context(|| format!("could not listen on {address}"))?;
            serve_tls(computers, listener, tls, token)
                .await
                .with_context(|| format!("serving TLS on {address}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn serve_takes_one_transport_and_only_tls_leaves_this_machine() {
        assert_eq!(parse(&words("serve --stdio")), Ok(Transport::Stdio));
        assert_eq!(
            parse(&words("serve --tcp 7070")),
            Ok(Transport::Tcp("127.0.0.1:7070".parse().unwrap()))
        );
        assert!(parse(&words("serve --tcp 0.0.0.0:7070")).is_err());
        assert_eq!(
            parse(&words("serve --tls 0.0.0.0:7070 /opt/glave/tls")),
            Ok(Transport::Tls(
                "0.0.0.0:7070".parse().unwrap(),
                PathBuf::from("/opt/glave/tls")
            ))
        );
        assert!(parse(&words("serve --tls 0.0.0.0:7070")).is_err());
        assert!(parse(&words("serve")).is_err());
        assert!(parse(&words("serve --stdio --tcp 1")).is_err());
        assert!(parse(&words("run")).is_err());
    }
}
