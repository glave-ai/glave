//! `glave connectors`: glave's catalog of connectors from the terminal, through the session API
//! of the person's server (`specs/session.md`, "Connectors"), as plain text: the connected ones
//! first, each with where it is used (each account beneath, for a service with more than one),
//! then the rest by category. `glave connect <id>` connects one, for every thread;
//! `glave connectors use <id> <where>` changes where it is used; `glave connectors add` connects
//! another account of one, named, for one place. An account is `<id>:<name>` (`stripe:Acme`).

use std::process::ExitCode;

use anyhow::bail;
use glave_ports::ProjectId;
use glave_session::protocol::{ConnectorAccount, ConnectorInfo, ConnectorStatus, Reply, Request};

use crate::connections::signed_in;
use crate::reach::{self, Reached};
use crate::tasks::run_with;

pub(crate) const USAGE: &str = "\
glave connectors: the apps glave can work in for you, each an official MCP server its maker
runs

Usage:
  glave connectors                        list them: those connected first, each account of
                                          one, then the rest by kind
  glave connectors use <id> everywhere    use one in every chat and project
  glave connectors use <id> <project>     use one only in that project, or only in Chats
  glave connectors add <id>:<name> <project>
                                          connect another account of one, called <name>, used
                                          only in that project: there it takes the place of
                                          the one used everywhere
  glave connectors disconnect <id>        disconnect one, destroying its sign-in

<id> is an app's, or one account's as <id>:<name> (stripe:Acme). Connect one with
glave connect <id>: you sign in to it in the browser, when it asks you to. It is used
everywhere until you choose where.";

/// `glave connectors ...`.
pub(crate) fn main(words: &[String]) -> ExitCode {
    run_with(words, USAGE, |words| async move {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            [] => reach::with(list).await,
            ["disconnect", id] => reach::with(async |server| disconnect(server, id).await).await,
            ["use", id, place @ ..] if !place.is_empty() => {
                let place = place.join(" ");
                reach::with(async |server| used(server, id, &place, false).await).await
            }
            ["add", id, place @ ..] if id.contains(':') && !place.is_empty() => {
                let place = place.join(" ");
                reach::with(async |server| used(server, id, &place, true).await).await
            }
            _ => bail!("unexpected words\n\n{USAGE}"),
        }
    })
}

/// The connector and the account `handle` names: `stripe`, or `stripe:Acme`.
fn account(handle: &str) -> (String, Option<String>) {
    match handle.split_once(':') {
        Some((id, name)) => (id.to_owned(), Some(name.to_owned())),
        None => (handle.to_owned(), None),
    }
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
        connected_lines(connector);
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

/// Prints a connector connected: its id, its name and where it is used, or, with more than
/// one account, how many, then each account by its handle with where it is used.
fn connected_lines(connector: &ConnectorInfo) {
    match connector.accounts.as_slice() {
        [one] if one.name.is_none() => {
            let (id, name) = (&connector.id, &connector.name);
            println!("  {id:<26} {name}: {}", one.used);
            again(connector, one);
        }
        accounts => {
            let (id, name, count) = (&connector.id, &connector.name, accounts.len());
            println!("  {id:<26} {name}: {count} accounts");
            for account in accounts {
                println!("    {:<24} {}", handle(connector, account), account.used);
                again(connector, account);
            }
        }
    }
}

/// How an account is named in these commands: `stripe`, or `stripe:Acme`.
fn handle(connector: &ConnectorInfo, account: &ConnectorAccount) -> String {
    match &account.name {
        Some(name) => format!("{}:{name}", connector.id),
        None => connector.id.clone(),
    }
}

/// Says how to sign in to `account` again, when its sign-in no longer works.
fn again(connector: &ConnectorInfo, account: &ConnectorAccount) {
    if account.status != ConnectorStatus::SignInAgain {
        return;
    }
    let handle = handle(connector, account);
    match account.name {
        Some(_) => println!("      sign in again with glave connectors add {handle} <where>"),
        None => println!("      sign in again with glave connect {handle}"),
    }
}

/// The project called `place` (their Chats among them), or none for everywhere.
async fn place_called(server: &mut Reached, place: &str) -> anyhow::Result<Option<ProjectId>> {
    if place.eq_ignore_ascii_case("everywhere") {
        return Ok(None);
    }
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
    Ok(Some(project.id))
}

/// Uses the account `handle` names from now on everywhere, or only in the project called
/// `place`; `adding` it, connects it first, signing in to it in the browser when it asks.
async fn used(server: &mut Reached, handle: &str, place: &str, adding: bool) -> anyhow::Result<()> {
    let only = place_called(server, place).await?;
    let (connector, account) = account(handle);
    let request = if adding {
        Request::ConnectorConnect {
            connector,
            account,
            only,
        }
    } else {
        Request::ConnectorPlace {
            connector,
            account,
            only,
        }
    };
    match server.request(request).await? {
        Reply::Connector { connector } => {
            connected_lines(&connector);
            Ok(())
        }
        Reply::SignIn { sign_in } => signed_in(server, &sign_in).await,
        _ => bail!("glave answered with something else"),
    }
}

/// Connects the connector `id` for every thread, signing in to it in the browser when it asks.
pub(crate) async fn connect(server: &mut Reached, id: &str) -> anyhow::Result<()> {
    let request = Request::ConnectorConnect {
        connector: id.to_owned(),
        account: None,
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

/// Disconnects the account `handle` names.
async fn disconnect(server: &mut Reached, handle: &str) -> anyhow::Result<()> {
    let (connector, account) = account(handle);
    let request = Request::ConnectorDisconnect { connector, account };
    server.request(request).await?;
    println!("Disconnected {handle}: its sign-in is destroyed.");
    Ok(())
}
