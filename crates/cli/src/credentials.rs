//! Git hosts on this computer (`specs/session.md`, "Git hosts"): the person connects GitHub,
//! signing in in the browser, or any git host with a token, from a client. glave checks the
//! token with the host, keeps it sealed in the store under a key of its own, and hands it to a
//! command's git or forge tool each time one asks (`asked.rs`).

mod asked;
mod check;
mod device;

use std::fmt::Write as _;
use std::sync::{Arc, PoisonError, RwLock};

use glave_config::policy::GithubFile;
use glave_event::{CredentialConnected, CredentialId, Forge, Principal, PrincipalId, Secret};
use glave_ports::auth::safe_to_send;
use glave_ports::{BoxFuture, CredentialHost};
use glave_session::protocol::{ApiError, CredentialInfo, DeviceCode, code, hint};
use glave_session::{GitHosts, NewCredential};

use crate::accounts::{Ledger, random};
use crate::data::Data;
use device::{Flows, Github, Grant, Polled};

/// The longest token or user name taken, in bytes.
const LONGEST: usize = 4096;

/// The person's git hosts: kept in the accounts ledger, offered to commands, and connected by
/// a token or by signing in to GitHub in the browser.
pub struct Hosts {
    ledger: Ledger,
    http: reqwest::Client,
    github: Option<Github>,
    flows: Flows,
    /// The hosts commands may ask about, as the ledger last held them.
    offered: RwLock<Vec<CredentialHost>>,
    /// Held while a token is renewed, so a refresh token is spent once.
    renewing: tokio::sync::Mutex<()>,
}

impl Hosts {
    /// The git hosts of `person` in `store`, when they may connect them here: everywhere but
    /// a server people sign in to with OIDC (`others_sign_in`), where an organization will
    /// grant them instead.
    pub async fn for_person(
        store: &Data,
        person: PrincipalId,
        github: Option<&GithubFile>,
        others_sign_in: bool,
    ) -> anyhow::Result<Option<Arc<Self>>> {
        if others_sign_in {
            return Ok(None);
        }
        let ledger = Ledger::open(store, person).await?;
        Self::open(ledger, github).await.map(Some)
    }

    /// The git hosts `ledger` keeps, signing in to GitHub through `github`'s app when set.
    async fn open(ledger: Ledger, github: Option<&GithubFile>) -> anyhow::Result<Arc<Self>> {
        let hosts = Self {
            ledger,
            http: check::client()?,
            github: github.map(Github::new),
            flows: Flows::default(),
            offered: RwLock::new(Vec::new()),
            renewing: tokio::sync::Mutex::new(()),
        };
        hosts.kept().await?;
        Ok(Arc::new(hosts))
    }

    /// The hosts kept now, offered to commands from now on.
    async fn kept(&self) -> anyhow::Result<Vec<CredentialConnected>> {
        let credentials = self.ledger.accounts().await?.credentials;
        let offered = credentials
            .iter()
            .map(|credential| CredentialHost {
                url: credential.url.clone(),
                forge: credential.forge,
            })
            .collect();
        *self.offered.write().unwrap_or_else(PoisonError::into_inner) = offered;
        Ok(credentials)
    }

    /// Keeps `credential`, replacing any for the same host, and offers it to commands.
    async fn keep(&self, credential: CredentialConnected) -> Result<CredentialInfo, ApiError> {
        self.ledger
            .keep_credential(&credential)
            .await
            .map_err(|e| failed(&e))?;
        self.kept().await.map_err(|e| failed(&e))?;
        Ok(info(&credential))
    }

    /// A new credential for `url` from what its host granted, made by the person here.
    fn made(
        &self,
        forge: Forge,
        url: String,
        username: String,
        login: Option<String>,
        grant: Grant,
    ) -> Result<CredentialConnected, ApiError> {
        let id = random()
            .and_then(|random| Ok(CredentialId::new(jiff::Timestamp::now(), random)?))
            .map_err(|e| failed(&e))?;
        Ok(CredentialConnected {
            credential: id,
            forge,
            url,
            username,
            token: grant.token,
            login,
            renewal: grant.renewal,
            by: Principal::person(self.ledger.person()),
        })
    }

    async fn connect(&self, new: NewCredential) -> Result<CredentialInfo, ApiError> {
        let url = origin(&new.url).ok_or_else(|| {
            invalid(
                "that is not a git host's address glave can use: give it as https://, such as \
                 https://github.com",
            )
        })?;
        let token = one_line(new.token.expose()).ok_or_else(|| {
            invalid("that does not look like a token: paste the whole token, and nothing else")
        })?;
        let username = match new.username {
            Some(name) => Some(
                one_line(&name)
                    .ok_or_else(|| invalid("a user name is one line, with no spaces"))?,
            ),
            None => None,
        };
        let token = Secret::new(token);
        let checked = check::check(&self.http, &url, &token).await?;
        let username = username
            .or_else(|| checked.login.clone())
            .unwrap_or_else(|| default_username(checked.forge).to_owned());
        let grant = Grant {
            token,
            renewal: None,
        };
        let credential = self.made(checked.forge, url, username, checked.login, grant)?;
        self.keep(credential).await
    }

    async fn poll(&self, flow: String) -> Result<Option<CredentialInfo>, ApiError> {
        let github = self.github.as_ref().ok_or_else(no_device)?;
        let Polled::Granted(grant) = github.poll(&self.http, &self.flows, &flow).await? else {
            return Ok(None);
        };
        let login = check::github_login(&self.http, &github.url, &grant.token).await?;
        let username = login
            .clone()
            .unwrap_or_else(|| default_username(Forge::Github).to_owned());
        let credential = self.made(Forge::Github, github.url.clone(), username, login, grant)?;
        self.keep(credential).await.map(Some)
    }
}

impl GitHosts for Hosts {
    fn connect(&self, new: NewCredential) -> BoxFuture<'_, Result<CredentialInfo, ApiError>> {
        Box::pin(Self::connect(self, new))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<CredentialInfo>, ApiError>> {
        Box::pin(async move {
            let kept = self.kept().await.map_err(|e| failed(&e))?;
            Ok(kept.iter().map(info).collect())
        })
    }

    fn remove(&self, credential: CredentialId) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            let removed = self.ledger.remove_credential(credential).await;
            if !removed.map_err(|e| failed(&e))? {
                return Err(ApiError::new(
                    code::NO_CREDENTIAL,
                    format!("no git host {credential} here"),
                ));
            }
            // Read again, so its token is destroyed now and commands no longer ask for it.
            self.kept().await.map_err(|e| failed(&e))?;
            Ok(())
        })
    }

    fn device(&self) -> bool {
        self.github.is_some()
    }

    fn install(&self) -> Option<String> {
        self.github.as_ref().and_then(Github::install)
    }

    fn start_device(&self) -> BoxFuture<'_, Result<DeviceCode, ApiError>> {
        Box::pin(async move {
            let github = self.github.as_ref().ok_or_else(no_device)?;
            let flow = random()
                .map(|bytes| {
                    bytes.iter().fold(String::new(), |mut flow, byte| {
                        let _ = write!(flow, "{byte:02x}");
                        flow
                    })
                })
                .map_err(|e| failed(&e))?;
            github.start(&self.http, &self.flows, flow).await
        })
    }

    fn poll_device(&self, flow: String) -> BoxFuture<'_, Result<Option<CredentialInfo>, ApiError>> {
        Box::pin(self.poll(flow))
    }
}

/// The host as a client may see it: never its token.
fn info(credential: &CredentialConnected) -> CredentialInfo {
    CredentialInfo {
        id: credential.credential,
        forge: credential.forge,
        url: credential.url.clone(),
        login: credential.login.clone(),
        hint: hint(credential.token.expose()),
    }
}

/// `url`'s origin, as git names the host, in lower case: `https://` and the host with any
/// port; `http://` only to this computer. None for anything else, or a URL with a user name,
/// a query or a fragment in it.
fn origin(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?.to_ascii_lowercase();
    let bad =
        host.is_empty() || host.contains(['@', '?', '#']) || url.contains(char::is_whitespace);
    let origin = format!("{}://{host}", scheme.to_ascii_lowercase());
    (!bad && safe_to_send(&origin, &[])).then_some(origin)
}

/// `text` without the spaces around it, when it is one line of visible characters.
fn one_line(text: &str) -> Option<String> {
    let text = text.trim();
    let fits = !text.is_empty() && text.len() <= LONGEST;
    (fits && !text.chars().any(|c| c.is_whitespace() || c.is_control())).then(|| text.to_owned())
}

/// The user name git sends with a token when the host did not say whose it is: the one each
/// forge documents for tokens.
fn default_username(forge: Forge) -> &'static str {
    match forge {
        Forge::Gitlab => "oauth2",
        Forge::Bitbucket => "x-token-auth",
        Forge::Github | Forge::Gitea | Forge::Other => "x-access-token",
    }
}

/// `-32602`, saying what to change.
fn invalid(message: &str) -> ApiError {
    ApiError::new(code::INVALID_PARAMS, message)
}

/// Signing in to GitHub in the browser is not set up here.
fn no_device() -> ApiError {
    ApiError::new(
        code::NOT_AVAILABLE,
        "signing in to GitHub in the browser is not set up here: connect it with a token instead",
    )
}

/// A failure of glave's own, such as the store's.
fn failed(error: &anyhow::Error) -> ApiError {
    ApiError::new(code::INTERNAL, format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_is_kept_as_its_origin() {
        assert_eq!(
            origin(" https://GitHub.com/acme/app.git ").as_deref(),
            Some("https://github.com")
        );
        assert_eq!(
            origin("http://127.0.0.1:3000").as_deref(),
            Some("http://127.0.0.1:3000")
        );
        for refused in [
            "http://gitlab.example.com",
            "https://me@github.com",
            "github.com",
            "https://",
            "ftp://github.com",
            "https://git hub.com",
        ] {
            assert_eq!(origin(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_token_is_one_line() {
        assert_eq!(one_line(" ghp_abc \n").as_deref(), Some("ghp_abc"));
        assert_eq!(one_line("two words"), None);
        assert_eq!(one_line(""), None);
        assert_eq!(one_line(&"x".repeat(LONGEST + 1)), None);
    }
}
