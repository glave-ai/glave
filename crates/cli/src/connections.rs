//! `glave connect` and `glave connections`: the person's git hosts from the terminal, through
//! the session API of the person's server (`reach.rs`, `specs/session.md`, "Git hosts"), as
//! the web app's settings reach them. A token is read from standard input, hidden on a
//! terminal, never from an argument, so it lands in neither the shell's history nor the list of
//! processes (`token.rs`).

mod token;

use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use glave_event::{CredentialId, Forge, Secret};
use glave_session::protocol::{Ability, CredentialInfo, Reply, Request};
use tokio::signal::unix::{SignalKind, signal};
use uuid::Uuid;

use crate::reach::{self, Reached};
use crate::tasks::run_with;

pub const CONNECT_USAGE: &str = "\
glave connect: connect a git host, so git and the host's own tool sign in as you

Usage:
  glave connect github          sign in to GitHub in the browser: glave shows a code and the
                                page to enter it on, and waits while you approve glave there.
                                Where that is not set up, connect GitHub with a token instead
  glave connect <address> [--username <name>]
                                connect a git host with a token, such as
                                glave connect https://gitlab.com. Paste the token when asked
                                (it does not show), or pipe it in. glave checks it with the
                                host and keeps it sealed; --username is the name git sends
                                with it, for a host that needs one

Connecting the same host again replaces it. List or remove them with glave connections.";

pub const CONNECTIONS_USAGE: &str = "\
glave connections: the git hosts you connected

Usage:
  glave connections               list them: the host, whom it signs in as, the end of its
                                  token, and what glave can do with it
  glave connections remove <id>   remove one, destroying its token";

/// GitHub's own address.
const GITHUB: &str = "https://github.com";

/// `glave connect ...`.
pub fn connect_main(words: &[String]) -> ExitCode {
    run_with(words, CONNECT_USAGE, |words| async move {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        let token = |word: &&str| *word == "--token" || token::looks_like(word);
        let (url, username) = match words.as_slice() {
            ["github"] => return reach::with(sign_in).await,
            words if words.iter().any(token) => bail!("{}", token::NOT_AN_ARGUMENT),
            [url] if !url.starts_with('-') => (*url, None),
            [url, "--username", name] => (*url, Some(*name)),
            _ => bail!("give a host's address, or github\n\n{CONNECT_USAGE}"),
        };
        let (url, token) = asked_token(url).await?;
        reach::with(async |server| connect(server, &url, token, username).await).await
    })
}

/// `glave connections ...`.
pub fn connections_main(words: &[String]) -> ExitCode {
    run_with(words, CONNECTIONS_USAGE, |words| async move {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        match words.as_slice() {
            [] => reach::with(list).await,
            ["remove", id] => {
                let credential = credential_id(id)?;
                reach::with(async |server| remove(server, credential, id).await).await
            }
            _ => bail!("unexpected words\n\n{CONNECTIONS_USAGE}"),
        }
    })
}

/// Signs in to GitHub in the browser, or connects it with a token where that is not set up.
async fn sign_in(server: &mut Reached) -> anyhow::Result<()> {
    let Reply::Credentials { device, .. } = server.request(Request::CredentialList {}).await?
    else {
        bail!("glave answered credential/list with something else");
    };
    if !device {
        eprintln!(
            "Signing in to GitHub in the browser is not set up here, so connect it with a token: \
             make one at {GITHUB}/settings/personal-access-tokens/new for the repositories glave \
             works on."
        );
        let (url, token) = asked_token(GITHUB).await?;
        return connect(server, &url, token, None).await;
    }
    let Reply::Device { device } = server.request(Request::CredentialDeviceStart {}).await? else {
        bail!("glave answered credential/device/start with something else");
    };
    println!("Open {} and enter the code {}", device.url, device.code);
    println!("Waiting while you approve glave there (Ctrl-C stops)...");
    loop {
        let flow = device.flow.clone();
        match server
            .request(Request::CredentialDevicePoll { flow })
            .await?
        {
            Reply::Credential { credential } => {
                break said_connected(server, &credential).await;
            }
            Reply::Waiting { .. } => {}
            _ => bail!("glave answered credential/device/poll with something else"),
        }
    }
}

/// The git host at `address`, as glave keeps it, and its token, read from standard input;
/// Ctrl-C stops it, the terminal as it was.
async fn asked_token(address: &str) -> anyhow::Result<(String, Secret)> {
    let Some(url) = crate::credentials::origin(address) else {
        bail!(
            "that is not a git host's address glave can use: give it as https:// and its host, \
             such as https://gitlab.com"
        );
    };
    // Listening before the terminal's echo goes off, so no Ctrl-C leaves it off.
    let mut interrupts = signal(SignalKind::interrupt()).context("could not listen for Ctrl-C")?;
    tokio::select! {
        token = token::read(&url) => Ok((url, token?)),
        _ = interrupts.recv() => bail!("stopped"),
    }
}

async fn connect(
    server: &mut Reached,
    url: &str,
    token: Secret,
    username: Option<&str>,
) -> anyhow::Result<()> {
    let request = Request::CredentialConnect {
        url: url.to_owned(),
        token,
        username: username.map(str::to_owned),
    };
    let Reply::Credential { credential } = server.request(request).await? else {
        bail!("glave answered credential/connect with something else");
    };
    said_connected(server, &credential).await
}

/// Says what was connected and what glave can do with it, and where to choose the
/// repositories GitHub's app may use.
async fn said_connected(server: &mut Reached, credential: &CredentialInfo) -> anyhow::Result<()> {
    println!("Connected {}", shown(credential));
    println!("  {}", can(credential));
    if credential.forge == Forge::Github
        && let Reply::Credentials {
            install: Some(install),
            ..
        } = server.request(Request::CredentialList {}).await?
    {
        println!("Choose the repositories glave may use at {install}");
    }
    Ok(())
}

async fn list(server: &mut Reached) -> anyhow::Result<()> {
    let Reply::Credentials {
        credentials,
        install,
        ..
    } = server.request(Request::CredentialList {}).await?
    else {
        bail!("glave answered credential/list with something else");
    };
    if credentials.is_empty() {
        println!("No git host is connected. Connect one with glave connect github, or");
        println!("glave connect <address>.");
    }
    for credential in &credentials {
        println!("{}", shown(credential));
        println!("  {}", can(credential));
        println!("  id {}", credential.id);
    }
    if let Some(install) = install
        && credentials
            .iter()
            .any(|credential| install.starts_with(&format!("{}/", credential.url)))
    {
        println!("Choose the repositories GitHub's app may use at {install}");
    }
    Ok(())
}

fn credential_id(id: &str) -> anyhow::Result<CredentialId> {
    let uuid = Uuid::parse_str(id).with_context(|| format!("{id:?} is not a connection's id"))?;
    CredentialId::try_from(uuid).with_context(|| format!("{id:?} is not a connection's id"))
}

async fn remove(server: &mut Reached, credential: CredentialId, id: &str) -> anyhow::Result<()> {
    server
        .request(Request::CredentialRemove { credential })
        .await?;
    println!("Removed {id}: its token is destroyed.");
    Ok(())
}

/// A connection as a line: the host, its forge, whom it signs in as and the end of its token.
fn shown(credential: &CredentialInfo) -> String {
    let host = credential.url.trim_start_matches("https://");
    let mut line = match credential.forge {
        _ if credential.url == GITHUB => "GitHub".to_owned(),
        Forge::Other => host.to_owned(),
        forge => format!("{host} ({})", forge_name(forge)),
    };
    if let Some(login) = &credential.login {
        let _ = write!(line, ", signed in as {login}");
    }
    if let Some(hint) = &credential.hint {
        let _ = write!(line, ", token ending {hint}");
    }
    line
}

/// What glave can do with a connection, in plain words; nothing claimed the host did not say.
fn can(credential: &CredentialInfo) -> String {
    let Some(can) = &credential.can else {
        return "What it can do depends on the access you gave its token.".to_owned();
    };
    if can.is_empty() {
        return "Can do nothing with your repositories yet.".to_owned();
    }
    let gitlab = credential.forge == Forge::Gitlab;
    let words: Vec<&str> = can
        .iter()
        .map(|ability| match ability {
            Ability::Read => "read and clone your repositories",
            Ability::Push => "push branches",
            Ability::PullRequests if gitlab => "open and comment on merge requests",
            Ability::PullRequests => "open and comment on pull requests",
            Ability::Issues => "open and comment on issues",
        })
        .collect();
    format!("Can {}.", words.join(", "))
}

fn forge_name(forge: Forge) -> &'static str {
    match forge {
        Forge::Github => "GitHub",
        Forge::Gitlab => "GitLab",
        Forge::Gitea => "Gitea",
        Forge::Bitbucket => "Bitbucket",
        Forge::Other => "git host",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(url: &str, forge: Forge, can: Option<Vec<Ability>>) -> CredentialInfo {
        CredentialInfo {
            id: CredentialId::new(
                jiff::Timestamp::from_second(1_760_000_000).unwrap(),
                [7; 10],
            )
            .unwrap(),
            forge,
            url: url.to_owned(),
            login: Some("ana".to_owned()),
            hint: Some("wxyz".to_owned()),
            can,
        }
    }

    #[test]
    fn a_connection_says_whom_it_signs_in_as_and_only_what_the_host_said_it_can_do() {
        let github = connection(GITHUB, Forge::Github, Some(vec![Ability::Read]));
        assert_eq!(
            shown(&github),
            "GitHub, signed in as ana, token ending wxyz"
        );
        assert_eq!(can(&github), "Can read and clone your repositories.");
        let gitlab = connection(
            "https://gitlab.example.com",
            Forge::Gitlab,
            Some(vec![Ability::Read, Ability::PullRequests]),
        );
        assert_eq!(
            shown(&gitlab),
            "gitlab.example.com (GitLab), signed in as ana, token ending wxyz"
        );
        assert_eq!(
            can(&gitlab),
            "Can read and clone your repositories, open and comment on merge requests."
        );
        let unsaid = connection("http://127.0.0.1:3000", Forge::Gitea, None);
        assert!(shown(&unsaid).starts_with("http://127.0.0.1:3000 (Gitea)"));
        assert_eq!(
            can(&unsaid),
            "What it can do depends on the access you gave its token."
        );
        let nothing = connection(GITHUB, Forge::Github, Some(Vec::new()));
        assert_eq!(can(&nothing), "Can do nothing with your repositories yet.");
    }
}
