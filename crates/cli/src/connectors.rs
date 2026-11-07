//! `glave connectors`: glave's catalog of connectors from the terminal, through the session API
//! of the person's server (`specs/session.md`, "Connectors"), as plain text: the connected ones
//! first, each with where it is used (each account beneath, for a service with more than one),
//! then the rest by category. `glave connect <id>` connects one, for every thread;
//! `glave connectors use <id> <where>` changes where it is used; `glave connectors add` connects
//! another account of one, named, for one place. An account is `<id>:<name>` (`stripe:Acme`).

use std::collections::BTreeMap;
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
                                          the one used everywhere. Also signs in to one again,
                                          where it is used
  glave connectors rename <id> <name>     call an account of one <name> from now on
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
            ["add", id, place @ ..] if !place.is_empty() => {
                let place = place.join(" ");
                reach::with(async |server| used(server, id, &place, true).await).await
            }
            ["rename", id, label @ ..] if !label.is_empty() => {
                let label = label.join(" ");
                reach::with(async |server| renamed(server, id, &label).await).await
            }
            _ => bail!("unexpected words\n\n{USAGE}"),
        }
    })
}

/// The connector, and the label of its account, `handle` names: `stripe`, or `stripe:Acme`.
fn labelled(handle: &str) -> (String, Option<String>) {
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
    let places = places(server, &connectors).await?;
    for connector in mine {
        connected_lines(connector, &places);
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
/// one account, how many, then each account by its handle with where it is used; each with who
/// it is signed in as, where its service said.
fn connected_lines(connector: &ConnectorInfo, places: &Places) {
    match connector.accounts.as_slice() {
        [one] => {
            let (handle, name) = (handle(connector, one), &connector.name);
            println!("  {handle:<26} {name}: {}", about(one));
            again(connector, one, places);
        }
        accounts => {
            let (id, name, count) = (&connector.id, &connector.name, accounts.len());
            println!("  {id:<26} {name}: {count} accounts");
            for labelled in accounts {
                println!("    {:<24} {}", handle(connector, labelled), about(labelled));
                again(connector, labelled, places);
            }
        }
    }
}

/// Where the account `labelled` is used, then who it is signed in as, where its service said.
fn about(labelled: &ConnectorAccount) -> String {
    match &labelled.signed_in {
        Some(signed_in) => format!("{} · {signed_in}", labelled.used),
        None => labelled.used.to_string(),
    }
}

/// The person's projects' names, by their ids, for saying how to sign in again where an
/// account is used: none when no account needs it.
type Places = BTreeMap<ProjectId, String>;

/// The names of the person's projects, when an account of `connectors` is to be signed in to
/// again, which its line says how to do where it is used.
async fn places(server: &mut Reached, connectors: &[ConnectorInfo]) -> anyhow::Result<Places> {
    let again = (connectors.iter().flat_map(|it| &it.accounts))
        .any(|it| it.status == ConnectorStatus::SignInAgain);
    if !again {
        return Ok(Places::new());
    }
    let Reply::Projects { projects, .. } =
        server.request(Request::ProjectList { limit: None }).await?
    else {
        bail!("glave answered project/list with something else");
    };
    Ok(projects.into_iter().map(|it| (it.id, it.name)).collect())
}

/// How an account is named in these commands, by the label the person gave it: `stripe`, or
/// `stripe:Acme`.
fn handle(connector: &ConnectorInfo, labelled: &ConnectorAccount) -> String {
    match &labelled.name {
        Some(label) => format!("{}:{label}", connector.id),
        None => connector.id.clone(),
    }
}

/// Says how to sign in to the account `labelled` again, where it is used, when its sign-in no
/// longer works.
fn again(connector: &ConnectorInfo, labelled: &ConnectorAccount, places: &Places) {
    if labelled.status != ConnectorStatus::SignInAgain {
        return;
    }
    let handle = handle(connector, labelled);
    let place = match labelled.only {
        None => "everywhere".to_owned(),
        Some(project) => match places.get(&project) {
            Some(name) => format!("{name:?}"),
            None => "<where>".to_owned(),
        },
    };
    println!("      sign in again with glave connectors add {handle} {place}");
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
    let (connector, label) = labelled(handle);
    let request = if adding {
        Request::ConnectorConnect {
            another: false,
            connector,
            account: label,
            only,
        }
    } else {
        Request::ConnectorPlace {
            connector,
            account: label,
            only,
        }
    };
    match server.request(request).await? {
        Reply::Connector { connector } => {
            let places = places(server, std::slice::from_ref(&connector)).await?;
            connected_lines(&connector, &places);
            Ok(())
        }
        Reply::SignIn { sign_in } => signed_in(server, &sign_in).await,
        _ => bail!("glave answered with something else"),
    }
}

/// Connects the connector `id` for every thread, signing in to it in the browser when it asks.
pub(crate) async fn connect(server: &mut Reached, id: &str) -> anyhow::Result<()> {
    let request = Request::ConnectorConnect {
        another: false,
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

/// Calls the account `handle` names `label` from now on.
async fn renamed(server: &mut Reached, handle: &str, label: &str) -> anyhow::Result<()> {
    let (connector, current) = labelled(handle);
    let request = Request::ConnectorRename {
        connector,
        account: current,
        name: Some(label.to_owned()),
    };
    let Reply::Connector { connector } = server.request(request).await? else {
        bail!("glave answered connector/rename with something else");
    };
    let places = places(server, std::slice::from_ref(&connector)).await?;
    connected_lines(&connector, &places);
    Ok(())
}

/// Disconnects the account `handle` names.
async fn disconnect(server: &mut Reached, handle: &str) -> anyhow::Result<()> {
    let (connector, label) = labelled(handle);
    let request = Request::ConnectorDisconnect {
        connector,
        account: label,
    };
    server.request(request).await?;
    println!("Disconnected {handle}: its sign-in is destroyed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use glave_session::protocol::ConnectorStatus;

    use super::*;

    #[test]
    fn an_account_says_where_it_is_used_and_who_it_is_signed_in_as() {
        let mut account = ConnectorAccount {
            name: Some("Acme".into()),
            only: None,
            used: "Everywhere, including chats".into(),
            status: ConnectorStatus::Connected,
            who: None,
            signed_in: None,
        };
        assert_eq!(about(&account), "Everywhere, including chats");
        account.signed_in = Some("Signed in as Acme Corp".into());
        assert_eq!(
            about(&account),
            "Everywhere, including chats · Signed in as Acme Corp"
        );
    }
}
