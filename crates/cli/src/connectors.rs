//! `glave connectors`: glave's catalog of connectors from the terminal, through the session API
//! of the person's server (`specs/session.md`, "Connectors"), as plain text: the connected ones
//! first, then the rest by category. `glave connect <id>` connects one.

use std::process::ExitCode;

use anyhow::bail;
use glave_session::protocol::{ConnectorInfo, ConnectorStatus, Reply, Request};

use crate::connections::signed_in;
use crate::reach::{self, Reached};
use crate::tasks::run_with;

pub(crate) const USAGE: &str = "\
glave connectors: the apps glave can work in for you, each an official MCP server its maker
runs

Usage:
  glave connectors                   list them: those connected first, then the rest by kind
  glave connectors disconnect <id>   disconnect one, destroying its sign-in

Connect one with glave connect <id>: you sign in to it in the browser, when it asks you to.";

/// `glave connectors ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            [] => reach::with(list).await,
            ["disconnect", id] => reach::with(async |server| disconnect(server, id).await).await,
            _ => bail!("unexpected words\n\n{USAGE}"),
        }
    })
}

/// Lists the connectors offered here: the connected ones, then the rest by category.
async fn list(server: &mut Reached) -> anyhow::Result<()> {
    let Reply::Connectors {
        categories,
        connectors,
    } = server.request(Request::ConnectorList {}).await?
    else {
        bail!("glave answered connector/list with something else");
    };
    let connected = |it: &&ConnectorInfo| it.status != ConnectorStatus::Available;
    let mine: Vec<&ConnectorInfo> = connectors.iter().filter(connected).collect();
    println!("Connected");
    if mine.is_empty() {
        println!("  none yet");
    }
    for connector in mine {
        println!("{}", line(connector));
        if connector.status == ConnectorStatus::SignInAgain {
            println!("    sign in again with glave connect {}", connector.id);
        }
    }
    for category in &categories {
        let rest: Vec<&ConnectorInfo> = connectors
            .iter()
            .filter(|it| it.category == category.id && it.status == ConnectorStatus::Available)
            .collect();
        if rest.is_empty() {
            continue;
        }
        println!("\n{}", category.name);
        for connector in rest {
            println!("{}", line(connector));
        }
    }
    println!("\nConnect one with glave connect <id>.");
    Ok(())
}

/// A connector as a line: its id, its name and what it reaches.
fn line(connector: &ConnectorInfo) -> String {
    format!(
        "  {:<26} {}: {}",
        connector.id, connector.name, connector.description
    )
}

/// Connects the connector `id`, signing in to it in the browser when it asks.
pub(crate) async fn connect(server: &mut Reached, id: &str) -> anyhow::Result<()> {
    let request = Request::ConnectorConnect {
        connector: id.to_owned(),
    };
    match server.request(request).await? {
        Reply::Connector { connector } => {
            println!("Connected {}", connector.name);
            Ok(())
        }
        Reply::SignIn { sign_in } => signed_in(server, &sign_in).await,
        _ => bail!("glave answered connector/connect with something else"),
    }
}

/// Disconnects the connector `id`.
async fn disconnect(server: &mut Reached, id: &str) -> anyhow::Result<()> {
    let request = Request::ConnectorDisconnect {
        connector: id.to_owned(),
    };
    server.request(request).await?;
    println!("Disconnected {id}: its sign-in is destroyed.");
    Ok(())
}
