//! Connections from the terminal: one of the person's MCP servers signed in to in the browser, by
//! its name or its address, listed with whether the sign-in still works, and removed. An
//! address none of their servers is at is a git host's, connected with a token.

use anyhow::bail;
use glave_session::protocol::{
    ApiError, ConnectionInfo, ConnectionStatus, Reply, Request, SignInLink, code,
};

use crate::reach::Reached;

/// Signs in to the server `named`, one of the person's MCP servers; else, for an address, to
/// the git host there with a token read from standard input.
pub(super) async fn server_or_host(server: &mut Reached, named: &str) -> anyhow::Result<()> {
    let start = Request::ConnectionStart {
        server: named.to_owned(),
    };
    match server.request(start).await {
        Ok(Reply::SignIn { sign_in }) => signed_in(server, &sign_in).await,
        Ok(_) => bail!("glave answered connection/start with something else"),
        Err(error) if answered(&error, code::NO_SERVER) => {
            let (url, token) = super::asked_token(named).await?;
            super::connect(server, &url, token, None).await
        }
        Err(error) => Err(error),
    }
}

/// Whether `error` is the server's answer with `code`.
fn answered(error: &anyhow::Error, code: i64) -> bool {
    error
        .downcast_ref::<ApiError>()
        .is_some_and(|error| error.code == code)
}

/// Shows the page to sign in on, and waits until the person has.
async fn signed_in(server: &mut Reached, sign_in: &SignInLink) -> anyhow::Result<()> {
    println!("Open {} to sign in", sign_in.url);
    println!("Waiting while you sign in there (Ctrl-C stops)...");
    loop {
        let flow = sign_in.flow.clone();
        match server.request(Request::ConnectionPoll { flow }).await? {
            Reply::Connection { connection } => {
                println!("Connected {}", shown(&connection));
                return Ok(());
            }
            Reply::Waiting { .. } => {}
            _ => bail!("glave answered connection/poll with something else"),
        }
    }
}

/// Lists the servers signed in to, and those that ask the person to sign in, where the server
/// keeps them.
pub(super) async fn list(server: &mut Reached) -> anyhow::Result<()> {
    let connections = match server.request(Request::ConnectionList {}).await {
        Ok(Reply::Connections { connections }) => connections,
        Ok(_) => bail!("glave answered connection/list with something else"),
        Err(error) if answered(&error, code::NOT_AVAILABLE) => return Ok(()),
        Err(error) => return Err(error),
    };
    if connections.is_empty() {
        println!("No connection is signed in to, and none of your MCP servers asks you to.");
    }
    for connection in &connections {
        println!("{}", shown(connection));
        let named = connection.name.as_ref().unwrap_or(&connection.url);
        match connection.status {
            ConnectionStatus::SignedIn => {}
            ConnectionStatus::Expired => println!("  Sign in again with glave connect {named}"),
            ConnectionStatus::SignedOut => println!("  Sign in with glave connect {named}"),
        }
    }
    Ok(())
}

/// Removes the sign-in to the server `named`, by its name or its address.
pub(super) async fn remove(server: &mut Reached, named: &str) -> anyhow::Result<()> {
    let remove = Request::ConnectionRemove {
        server: named.to_owned(),
    };
    server.request(remove).await?;
    println!("Removed {named}: its sign-in is destroyed.");
    Ok(())
}

/// A connection as a line: its name, its address, and whether the person is still signed in.
fn shown(connection: &ConnectionInfo) -> String {
    let status = match connection.status {
        ConnectionStatus::SignedIn => "signed in",
        ConnectionStatus::Expired => "signed out: its sign-in expired",
        ConnectionStatus::SignedOut => "not signed in",
    };
    match &connection.name {
        Some(name) => format!("{name} ({}), {status}", connection.url),
        None => format!("{}, {status}", connection.url),
    }
}
