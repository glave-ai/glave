//! Checking a token when the person connects a git host: one small authenticated call where
//! the host has one, which also tells which forge it runs and whom the token is for. GitHub,
//! GitLab and Bitbucket are known by name; any other host is asked as GitLab, Gitea (or
//! Forgejo) and GitHub Enterprise answer, and a host none of them answers for is kept as it is,
//! unchecked, for git to try.

use std::time::Duration;

use glave_event::{Forge, Secret};
use glave_session::protocol::{ApiError, code};
use serde_json::Value;

use super::abilities;

/// How long one call may take.
const CALL: Duration = Duration::from_secs(15);
/// The most of an answer read, in bytes.
const MOST: usize = 256 * 1024;

/// What the host said of a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Checked {
    pub forge: Forge,
    /// Whom the token is for, when the host said.
    pub login: Option<String>,
    /// What the token may do, when the host said.
    pub scopes: Option<Vec<String>>,
}

/// The clients every call to a git host is made with: no redirects, so a token never follows
/// one to another host, and no proxy for plain http, which reaches only this computer, so a
/// token never crosses a proxy in the clear.
#[derive(Clone)]
pub(super) struct Http {
    through_proxy: reqwest::Client,
    direct: reqwest::Client,
}

impl Http {
    pub(super) fn new() -> anyhow::Result<Self> {
        let builder = || {
            reqwest::Client::builder()
                .timeout(CALL)
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("glave/", env!("CARGO_PKG_VERSION")))
        };
        Ok(Self {
            through_proxy: builder().build()?,
            direct: builder().no_proxy().build()?,
        })
    }

    /// The client for a call to `url`.
    pub(super) fn to(&self, url: &str) -> &reqwest::Client {
        if url.starts_with("http://") {
            &self.direct
        } else {
            &self.through_proxy
        }
    }
}

/// How a host's API takes a token.
#[derive(Debug, Clone, Copy)]
enum Header {
    Bearer,
    /// GitLab's own header.
    PrivateToken,
    /// Gitea's and Forgejo's `token` scheme.
    Token,
}

/// One way a forge's API says whom a token is for.
struct Probe {
    forge: Forge,
    url: String,
    header: Header,
    /// The field of the answer naming the account.
    login: &'static str,
}

/// What one probe found.
enum Found {
    /// The forge answered: a token it knows, perhaps without leave to say whose, and what it
    /// may do when the answer said.
    Yes(Option<String>, Option<Vec<String>>),
    /// It refused the token.
    Refused,
    /// Not this forge.
    No,
    /// The host could not be reached.
    Unreachable,
}

/// Checks `token` with the host at `url` (its origin, as connecting keeps it).
pub(super) async fn check(http: &Http, url: &str, token: &Secret) -> Result<Checked, ApiError> {
    let name = url.split_once("://").map_or(url, |(_, host)| host);
    let mut refused = false;
    let mut reached = false;
    for probe in probes(url) {
        match ask(http, &probe, token).await {
            Found::Yes(login, scopes) => {
                let scopes = match probe.forge {
                    Forge::Gitlab => abilities::gitlab_scopes(http, url, token).await,
                    _ => scopes,
                };
                return Ok(Checked {
                    forge: probe.forge,
                    login,
                    scopes,
                });
            }
            Found::Refused => {
                (refused, reached) = (true, true);
            }
            Found::No => reached = true,
            Found::Unreachable => {}
        }
    }
    if refused {
        return Err(ApiError::new(
            code::HOST,
            format!(
                "the git host at {name} did not accept that token: check that you copied all of \
                 it and that it is still active"
            ),
        ));
    }
    if !reached {
        return Err(ApiError::new(
            code::HOST,
            format!("glave could not reach {name}: check the address and your connection"),
        ));
    }
    Ok(Checked {
        forge: Forge::Other,
        login: None,
        scopes: None,
    })
}

/// The calls that tell whom a token is for at `url`, in the order tried.
fn probes(url: &str) -> Vec<Probe> {
    let probe = |forge, url: String, header, login| Probe {
        forge,
        url,
        header,
        login,
    };
    match url {
        "https://github.com" => vec![probe(
            Forge::Github,
            "https://api.github.com/user".into(),
            Header::Bearer,
            "login",
        )],
        "https://bitbucket.org" => vec![probe(
            Forge::Bitbucket,
            "https://api.bitbucket.org/2.0/user".into(),
            Header::Bearer,
            "username",
        )],
        _ => {
            let gitlab = probe(
                Forge::Gitlab,
                format!("{url}/api/v4/user"),
                Header::PrivateToken,
                "username",
            );
            if url == "https://gitlab.com" {
                return vec![gitlab];
            }
            vec![
                gitlab,
                probe(
                    Forge::Gitea,
                    format!("{url}/api/v1/user"),
                    Header::Token,
                    "login",
                ),
                github_api(url),
            ]
        }
    }
}

/// GitHub's API at `url`: `api.github.com` for GitHub itself, `<url>/api/v3` for GitHub
/// Enterprise Server.
fn github_api(url: &str) -> Probe {
    Probe {
        forge: Forge::Github,
        url: format!("{}/user", api(url)),
        header: Header::Bearer,
        login: "login",
    }
}

/// Where GitHub at `url` has its API.
pub(super) fn api(url: &str) -> String {
    if url == "https://github.com" {
        "https://api.github.com".to_owned()
    } else {
        format!("{url}/api/v3")
    }
}

/// Whom GitHub at `url` says `token` is for: the account a sign-in in the browser connected.
pub(super) async fn github_login(
    http: &Http,
    url: &str,
    token: &Secret,
) -> Result<Option<String>, ApiError> {
    match ask(http, &github_api(url), token).await {
        Found::Yes(login, _) => Ok(login),
        _ => Err(ApiError::new(
            code::HOST,
            "GitHub did not accept the sign-in: start again",
        )),
    }
}

/// Asks one probe about `token`.
async fn ask(http: &Http, probe: &Probe, token: &Secret) -> Found {
    let request = http
        .to(&probe.url)
        .get(&probe.url)
        .header(reqwest::header::ACCEPT, "application/json");
    let request = match probe.header {
        Header::Bearer => request.bearer_auth(token.expose()),
        Header::PrivateToken => request.header("PRIVATE-TOKEN", token.expose()),
        Header::Token => request.header(
            reqwest::header::AUTHORIZATION,
            format!("token {}", token.expose()),
        ),
    };
    let Ok(response) = request.send().await else {
        return Found::Unreachable;
    };
    let scopes = match probe.forge {
        Forge::Github => abilities::github_scopes(response.headers(), token),
        _ => None,
    };
    match response.status().as_u16() {
        // A name the host gives is kept only as one line, as a name the person gives is.
        200 => match read(response).await {
            Some(Value::Object(fields)) => Found::Yes(
                fields
                    .get(probe.login)
                    .and_then(Value::as_str)
                    .and_then(super::one_line),
                scopes,
            ),
            _ => Found::No,
        },
        401 => Found::Refused,
        // A token the forge knows, without leave to say whose (a GitLab token for git only).
        403 if read(response).await.is_some() => Found::Yes(None, None),
        _ => Found::No,
    }
}

/// The answer's JSON, read up to [`MOST`] bytes; none when it is not JSON.
pub(super) async fn read(mut response: reqwest::Response) -> Option<Value> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        body.extend_from_slice(&chunk);
        if body.len() > MOST {
            return None;
        }
    }
    serde_json::from_slice(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hosts_are_asked_once_and_others_as_each_forge() {
        let forges = |url: &str| -> Vec<Forge> { probes(url).iter().map(|p| p.forge).collect() };
        assert_eq!(forges("https://github.com"), [Forge::Github]);
        assert_eq!(forges("https://gitlab.com"), [Forge::Gitlab]);
        assert_eq!(forges("https://bitbucket.org"), [Forge::Bitbucket]);
        assert_eq!(
            forges("https://git.example.com"),
            [Forge::Gitlab, Forge::Gitea, Forge::Github]
        );
        assert_eq!(
            github_api("https://git.example.com").url,
            "https://git.example.com/api/v3/user"
        );
    }
}
