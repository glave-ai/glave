//! `glave connectors`: glave's catalog of connectors from the terminal, through the session API
//! of the person's server (`specs/session.md`, "Connectors"), as plain text: the connected ones
//! first, each with where it is used, then the rest by category. `glave connect <id>` connects
//! one, for every thread; `glave connectors use <id> <where>` changes where it is used.

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
  glave connectors                       list them: those connected first, then the rest by kind
  glave connectors use <id> everywhere   use one in every chat and project
  glave connectors use <id> <project>    use one only in that project, or only in Chats
  glave connectors disconnect <id>       disconnect one, destroying its sign-in

Connect one with glave connect <id>: you sign in to it in the browser, when it asks you to.
It is used everywhere until you choose where.";

/// `glave connectors ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            [] => reach::with(list).await,
            ["disconnect", id] => reach::with(async |server| disconnect(server, id).await).await,
            ["use", id, place @ ..] if !place.is_empty() => {
                let place = place.join(" ");
                reach::with(async |server| used(server, id, &place).await).await
            }
            _ => bail!("unexpected words\n\n{USAGE}"),
        }
    })
}

/// Lists the connectors offered here: the connected ones, then the rest by category.
async fn list(server: &mut Reached) -> anyhow::Result<()> {
    let Reply::Connectors {
        categories,
        connectors,
        ..
    } = server
        .request(Request::ConnectorList {
            company: None,
            category: None,
            tag: None,
            used_in: None,
        })
        .await?
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
        println!("{}", used_line(connector));
        if connector.status == ConnectorStatus::SignInAgain {
            println!("    sign in again with glave connect {}", connector.id);
        }
    }
    for category in &categories {
        let rest: Vec<&ConnectorInfo> = connectors
            .iter()
            .filter(|it| {
                it.categories.first() == Some(&category.id)
                    && it.status == ConnectorStatus::Available
            })
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

/// A connector connected as a line: its id, its name and where it is used.
fn used_line(connector: &ConnectorInfo) -> String {
    let used = connector.used.as_ref().map(ToString::to_string);
    format!(
        "  {:<26} {}: {}",
        connector.id,
        connector.name,
        used.unwrap_or_default()
    )
}

/// Uses the connector `id` from now on everywhere, or only in the project called `place`
/// (their Chats among them).
async fn used(server: &mut Reached, id: &str, place: &str) -> anyhow::Result<()> {
    let only = if place.eq_ignore_ascii_case("everywhere") {
        None
    } else {
        let Reply::Projects { projects, .. } =
            server.request(Request::ProjectList { limit: None }).await?
        else {
            bail!("glave answered project/list with something else");
        };
        let named = projects
            .iter()
            .find(|it| it.name.eq_ignore_ascii_case(place));
        let Some(project) = named else {
            bail!("none of your projects is called {place:?}: say everywhere, or a project's name");
        };
        Some(project.id)
    };
    let request = Request::ConnectorPlace {
        connector: id.to_owned(),
        only,
    };
    match server.request(request).await? {
        Reply::Connector { connector } => {
            println!("{}", used_line(&connector).trim_start());
            Ok(())
        }
        _ => bail!("glave answered connector/place with something else"),
    }
}

/// Connects the connector `id` for every thread, signing in to it in the browser when it asks.
pub(crate) async fn connect(server: &mut Reached, id: &str) -> anyhow::Result<()> {
    let request = Request::ConnectorConnect {
        connector: id.to_owned(),
        only: None,
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
