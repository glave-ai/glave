//! Git hosts on this computer (`specs/session.md`, "Git hosts"): the person connects GitHub,
//! signing in in the browser, or any git host with a token, from a client. glave checks the
//! token with the host, keeps it sealed in the store under a key of its own, and hands it to a
//! command's git or forge tool each time one asks (`asked.rs`).

mod abilities;
mod asked;
mod check;
mod device;
pub(crate) mod renewing;

use std::fmt::Write as _;
use std::sync::{Arc, PoisonError, RwLock};

use glave_config::policy::GithubFile;
use glave_event::{CredentialConnected, CredentialId, Forge, Principal, PrincipalId, Secret};
use glave_ports::computer::CredentialHost;
use glave_ports::{BoxFuture, Clock as _, SystemClock};
use glave_session::protocol::{ApiError, CredentialInfo, DeviceCode, code, hint};
use glave_session::{GitHosts, NewCredential};

use crate::accounts::{Ledger, random};
use crate::data::Data;
use check::{Checked, Http};
use device::{Flows, Github, Grant, Polled};
use renewing::Renewals;

/// The longest token or user name taken, in bytes.
const LONGEST: usize = 4096;

/// The person's git hosts: kept in the accounts ledger, offered to commands, and connected by
/// a token or by signing in to GitHub in the browser.
pub(crate) struct Hosts {
    ledger: Ledger,
    http: Http,
    github: Option<Github>,
    flows: Flows,
    /// The hosts commands may ask about, as the ledger last held them.
    offered: RwLock<Vec<CredentialHost>>,
    renewals: Renewals<CredentialConnected>,
}

impl Hosts {
    /// The git hosts of `person` in `store`, when they may connect them here: everywhere but
    /// a server people sign in to with OIDC (`others_sign_in`), where an organization will
    /// grant them instead.
    pub(crate) async fn for_person(
        store: &Data,
        person: PrincipalId,
        github: Option<&GithubFile>,
        others_sign_in: bool,
    ) -> anyhow::Result<Option<Arc<Self>>> {
        if others_sign_in {
            return Ok(None);
        }
        let ledger = crate::accounts::open(store, person).await?;
        Self::open(ledger, github).await.map(Some)
    }

    /// The git hosts `ledger` keeps, signing in to GitHub through `github`'s app when set.
    async fn open(ledger: Ledger, github: Option<&GithubFile>) -> anyhow::Result<Arc<Self>> {
        let hosts = Self {
            ledger,
            http: Http::new()?,
            github: github.and_then(Github::new),
            flows: Flows::default(),
            offered: RwLock::new(Vec::new()),
            renewals: Renewals::default(),
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
        self.ledger.keep_credential(&credential).await?;
        self.kept().await.map_err(|e| ApiError::internal(&e))?;
        Ok(self.described(&credential).await)
    }

    /// A new credential for `url` from what its host granted and said of it, made by the person
    /// here.
    fn made(
        &self,
        url: String,
        username: String,
        checked: Checked,
        grant: Grant,
    ) -> Result<CredentialConnected, ApiError> {
        Ok(CredentialConnected {
            credential: new_id().map_err(|e| ApiError::internal(&e))?,
            forge: checked.forge,
            url,
            username,
            token: grant.token,
            login: checked.login,
            scopes: checked.scopes,
            renewal: grant.renewal,
            by: Principal::person(self.ledger.person()),
        })
    }

    /// The host as a client may see it, never its token, with what glave can do with it now.
    async fn described(&self, credential: &CredentialConnected) -> CredentialInfo {
        let can = abilities::now(&self.http, credential).await;
        CredentialInfo {
            id: credential.credential,
            forge: credential.forge,
            url: credential.url.clone(),
            login: credential.login.clone(),
            hint: hint(credential.token.expose()),
            can,
        }
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
        let credential = self.made(url, username, checked, grant)?;
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
        let checked = Checked {
            forge: Forge::Github,
            login,
            scopes: None,
        };
        let credential = self.made(github.url.clone(), username, checked, grant)?;
        self.keep(credential).await.map(Some)
    }
}

impl GitHosts for Hosts {
    fn connect(&self, new: NewCredential) -> BoxFuture<'_, Result<CredentialInfo, ApiError>> {
        Box::pin(Self::connect(self, new))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<CredentialInfo>, ApiError>> {
        Box::pin(async move {
            let kept = self.kept().await.map_err(|e| ApiError::internal(&e))?;
            let mut described = Vec::with_capacity(kept.len());
            for credential in &kept {
                described.push(self.described(credential).await);
            }
            Ok(described)
        })
    }

    fn remove(&self, credential: CredentialId) -> BoxFuture<'_, Result<(), ApiError>> {
        Box::pin(async move {
            if !self.ledger.remove_credential(credential).await? {
                return Err(ApiError::new(
                    code::NO_CREDENTIAL,
                    format!("no git host {credential} here"),
                ));
            }
            // Read again, so its token is destroyed now and commands no longer ask for it.
            self.kept().await.map_err(|e| ApiError::internal(&e))?;
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
            let flow = new_flow().map_err(|e| ApiError::internal(&e))?;
            github.start(&self.http, &self.flows, flow).await
        })
    }

    fn poll_device(&self, flow: String) -> BoxFuture<'_, Result<Option<CredentialInfo>, ApiError>> {
        Box::pin(self.poll(flow))
    }
}

/// `url`'s origin, as git names the host and glave offers it to commands
/// ([`CredentialHost::named`]): `https://` and the host with any port, `http://` only to this
/// computer, in lower case. None for anything else, or a URL with a user name, a query or a
/// fragment before its path.
pub(crate) fn origin(url: &str) -> Option<String> {
    let url = url.trim();
    if url.contains(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?;
    CredentialHost::named(&format!("{scheme}://{host}"), Forge::Other).map(|named| named.url)
}

/// A new sign-in's id: a git host's, or a connection's.
pub(crate) fn new_id() -> anyhow::Result<CredentialId> {
    Ok(CredentialId::new(SystemClock.now(), random()?)?)
}

/// A new id for a sign-in waiting in the browser, which clients name it by: random, in hex.
pub(crate) fn new_flow() -> anyhow::Result<String> {
    Ok(random()?.iter().fold(String::new(), |mut flow, byte| {
        let _ = write!(flow, "{byte:02x}");
        flow
    }))
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

#[cfg(test)]
mod tests {
    use glave_ports::testing::FAKE_SECRET;

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
            "https://git_hub.com",
            "https://github.com?x=1",
            "https://gïthub.com",
            "https://github.com%2fevil",
        ] {
            assert_eq!(origin(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_token_is_one_line() {
        let pasted = format!(" {FAKE_SECRET} \n");
        assert_eq!(one_line(&pasted).as_deref(), Some(FAKE_SECRET));
        assert_eq!(one_line("two words"), None);
        assert_eq!(one_line(""), None);
        assert_eq!(one_line(&"x".repeat(LONGEST + 1)), None);
    }
}
