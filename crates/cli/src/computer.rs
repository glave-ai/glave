//! `glave computer serve`: this machine as a computer for a harness elsewhere, over the
//! computer protocol (`specs/computers.md`) on stdio or on TCP at 127.0.0.1.

use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context as _, bail};
use glave_computer::{LocalComputers, serve, serve_tcp};
use glave_harness::is_secret_variable;
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

Copies of repositories are kept in $GLAVE_HOME/workspaces. Commands get glave's environment
without anything named like a key, token, secret or password.";

/// How to serve.
#[derive(Debug, PartialEq, Eq)]
enum Transport {
    Stdio,
    Tcp(SocketAddr),
}

fn parse(arguments: &[String]) -> Result<Transport, String> {
    match arguments {
        [serve, stdio] if serve == "serve" && stdio == "--stdio" => Ok(Transport::Stdio),
        [serve, tcp, address] if serve == "serve" && tcp == "--tcp" => {
            let address = match address.parse::<u16>() {
                Ok(port) => SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                Err(_) => address
                    .parse()
                    .map_err(|_| format!("--tcp {address:?} is not a port or host:port"))?,
            };
            if address.ip().is_loopback() {
                Ok(Transport::Tcp(address))
            } else {
                Err(format!(
                    "--tcp {address} is not on this machine; serving computers across machines \
                     needs TLS, which comes later"
                ))
            }
        }
        _ => Err("expected: glave computer serve --stdio, or --tcp <port>".into()),
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

async fn run(transport: Transport) -> anyhow::Result<()> {
    let environment = std::env::vars_os()
        .filter(|(name, _)| !is_secret_variable(name))
        .collect();
    let computers = Arc::new(LocalComputers::new(
        home::folder()?.join("workspaces"),
        environment,
    ));
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
                .filter(|token| token.len() >= 16)
            else {
                bail!(
                    "glave computer serve --tcp needs GLAVE_COMPUTER_TOKEN, at least 16 \
                     characters: a listener never runs without one"
                );
            };
            let listener = TcpListener::bind(address)
                .await
                .with_context(|| format!("could not listen on {address}"))?;
            serve_tcp(computers, listener, token)
                .await
                .with_context(|| format!("serving on {address}"))
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
    fn serve_takes_one_transport_on_this_machine() {
        assert_eq!(parse(&words("serve --stdio")), Ok(Transport::Stdio));
        assert_eq!(
            parse(&words("serve --tcp 7070")),
            Ok(Transport::Tcp("127.0.0.1:7070".parse().unwrap()))
        );
        assert!(parse(&words("serve --tcp 0.0.0.0:7070")).is_err());
        assert!(parse(&words("serve")).is_err());
        assert!(parse(&words("serve --stdio --tcp 1")).is_err());
        assert!(parse(&words("run")).is_err());
    }
}
