//! The CLI as a client of a session server elsewhere (`glave --server https://…`, or
//! `[server] url` in the person's `config.toml`): threads, turns, goals, approvals and
//! questions go over HTTP and are followed exactly as here. Nothing runs on this computer but
//! the terminal: the server has the model, the tools and the history. Sign-in is a token in
//! GLAVE_SERVER_TOKEN: an OIDC access token for the server's audience, or, on the server's own
//! computer, its local token. The token goes only to that server.

use std::io::IsTerminal as _;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use glave_harness::{DEFAULT_BUDGET, Goal};
use glave_session::client::Remote;
use glave_session::protocol::{Reply, Request};

use crate::args::Args;
use crate::client::{Client, Session};
use crate::home;
use crate::input::Input;
use crate::session::{converse, export, pick_thread};
use crate::terminal::Printer;

/// Where the token that signs this client in comes from.
pub const TOKEN: &str = "GLAVE_SERVER_TOKEN";

/// The server this run is a client of: `--server`, else the person's own `[server] url`.
pub async fn chosen(args: &Args) -> anyhow::Result<Option<String>> {
    if let Some(url) = &args.server {
        return Ok(Some(url.clone()));
    }
    let path = home::folder()?.join(glave_config::policy::CONFIG);
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => Ok(glave_config::policy::config(&text, &path)?
            .server
            .map(|server| server.url)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}

/// Runs the turn, goal or session `args` ask for on the server at `url`.
pub async fn run(args: &Args, url: &str) -> anyhow::Result<ExitCode> {
    if let Some(flag) = local_only(args) {
        bail!(
            "{flag} chooses how glave runs on this computer: not with --server, whose server chooses"
        );
    }
    let Some(token) = std::env::var(TOKEN).ok().filter(|token| !token.is_empty()) else {
        bail!(
            "to use the server at {url}, sign in: set {TOKEN} to an access token for it (sign-in \
             in the terminal is not available yet)"
        );
    };
    let remote = Remote::new(url, token)?;
    let _telemetry = crate::telemetry::start(args.log.as_deref(), None)?;
    let elsewhere = Session::Elsewhere(&remote);
    let Reply::Server(_) = elsewhere.request(Request::Discover {}).await? else {
        bail!("{url} is not a glave session server");
    };
    let worktree = args.worktree.then_some(true);
    let (thread, note) = pick_thread(elsewhere, args.resume, worktree).await?;
    let input = Input::start();
    let client = Client {
        server: elsewhere,
        thread,
        input: &input,
        endpoint: None,
        asks: std::io::stdin().is_terminal(),
        servers: None,
    };
    let mut printer = Printer::new("glave", format!("on {}", remote.url()));
    let code = if let Some(objective) = &args.goal {
        let goal = Goal {
            objective: objective.clone(),
            token_budget: args.goal_budget.unwrap_or(DEFAULT_BUDGET),
        };
        Box::pin(client.goal(goal, &mut printer)).await
    } else if args.prompt.is_empty() {
        printer.banner_elsewhere(remote.url(), note);
        Box::pin(converse(&client, &mut printer)).await
    } else {
        Box::pin(client.turn(&args.prompt, &mut printer)).await
    };
    if let Some(path) = &args.events {
        export(elsewhere, thread, path).await?;
    }
    Ok(code)
}

/// The first flag `args` has that only means something for glave running here.
fn local_only(args: &Args) -> Option<&'static str> {
    [
        (args.model.is_some(), "--model"),
        (args.approval.is_some(), "--approval"),
        (args.sandbox.is_some(), "--sandbox"),
        (args.network.is_some(), "--network"),
        (args.on.is_some(), "--on"),
        (args.max_steps.is_some(), "--max-steps"),
        (args.background, "--background"),
    ]
    .into_iter()
    .find_map(|(given, flag)| given.then_some(flag))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_only_this_computer_chooses_is_refused_with_a_server() {
        let args = Args {
            model: Some("m".into()),
            ..Args::default()
        };
        assert_eq!(local_only(&args), Some("--model"));
        let args = Args {
            prompt: "hi".into(),
            resume: true,
            worktree: true,
            ..Args::default()
        };
        assert_eq!(local_only(&args), None);
    }
}
