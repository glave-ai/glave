//! Model accounts on this computer (`specs/session.md`, "Model accounts"): the person connects
//! their key for OpenAI, Anthropic, OpenRouter or another OpenAI-compatible server from a
//! client, with no settings to write. glave checks the key with the service, keeps it sealed in
//! the store under a key of its own, and runs the account's models from then on, beside any
//! the settings choose, which stay first.

mod check;
mod lineup;

use std::sync::{Mutex, PoisonError};

use glave_event::{Principal, ProviderConnected, ProviderId, ProviderKind, Secret};
use glave_ports::{BoxFuture, Failure, ProviderError};
use glave_provider_openai::Config;
use glave_session::protocol::{ApiError, ErrorData, ProviderInfo, code, hint};
use glave_session::{Models, NewProvider, Providers};

use crate::accounts::{Ledger, random};
use crate::config::Endpoint;
use crate::gateway::Switch;
use crate::model;

pub use lineup::{Built, Lineup, endpoint};

/// The longest key taken, in bytes.
const LONGEST_KEY: usize = 4096;

/// Where a fix for a refused key is made, for `config::fix`.
const KEY_FROM: &str = "the key of the model account you connected";

/// What says where an account's models are, for `config::fix`.
const AT: &str = "the model account you connected";

/// The person's model accounts, and the gateway their models run on.
pub struct Host {
    ledger: Ledger,
    /// What the models are built from: replaced when the person's settings change.
    lineup: Mutex<Lineup>,
    switch: Switch,
}

impl Host {
    pub fn new(ledger: Ledger, lineup: Lineup, switch: Switch) -> Self {
        Self {
            ledger,
            lineup: Mutex::new(lineup),
            switch,
        }
    }

    /// What the models are built from now.
    fn lineup(&self) -> Lineup {
        self.lineup
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Builds the models from `lineup` from now on, with the accounts kept, and sends calls
    /// to them.
    pub async fn relined(&self, lineup: Lineup) -> anyhow::Result<Models> {
        let accounts = self.ledger.accounts().await?;
        let built = lineup.build(&accounts.providers)?;
        *self.lineup.lock().unwrap_or_else(PoisonError::into_inner) = lineup;
        self.switch.set(built.gateway);
        Ok(built.models)
    }

    /// Builds the models of the accounts kept now, and sends calls to them.
    async fn rebuilt(&self) -> Result<Models, ApiError> {
        let accounts = self.ledger.accounts().await.map_err(|e| failed(&e))?;
        let built = self
            .lineup()
            .build(&accounts.providers)
            .map_err(|e| failed(&e))?;
        self.switch.set(built.gateway);
        Ok(built.models)
    }

    async fn connect(&self, new: NewProvider) -> Result<(ProviderInfo, Models), ApiError> {
        let url = checked_url(new.kind, new.url)?;
        let key = checked_key(&new.key)?;
        let mut account = ProviderConnected {
            provider: random()
                .and_then(|random| Ok(ProviderId::new(jiff::Timestamp::now(), random)?))
                .map_err(|e| failed(&e))?,
            kind: new.kind,
            url,
            key,
            models: Vec::new(),
            by: Principal::person(self.ledger.person()),
        };
        let endpoint = endpoint_of(&account, "").ok_or_else(|| invalid(NOT_AN_ADDRESS))?;
        let models = check::models(new.kind, &endpoint)
            .await
            .map_err(|error| refused(&account, &error))?;
        if models.is_empty() {
            return Err(provider_failed(
                Failure::UnknownModel,
                format!(
                    "{} offers no models glave can talk with on this account",
                    service(&account)
                ),
            ));
        }
        account.models = models;
        // Every account glave keeps must line up with the rest, so a start never fails on one.
        let mut accounts = self
            .ledger
            .accounts()
            .await
            .map_err(|e| failed(&e))?
            .providers;
        accounts.push(account.clone());
        self.lineup().build(&accounts).map_err(|e| {
            ApiError::new(
                code::NOT_ALLOWED,
                format!("glave cannot run this account's models here: {e:#}"),
            )
        })?;
        self.ledger
            .keep_provider(&account)
            .await
            .map_err(|e| failed(&e))?;
        Ok((info(&account), self.rebuilt().await?))
    }
}

impl Providers for Host {
    fn connect(&self, new: NewProvider) -> BoxFuture<'_, Result<(ProviderInfo, Models), ApiError>> {
        Box::pin(Self::connect(self, new))
    }

    fn list(&self) -> BoxFuture<'_, Result<Vec<ProviderInfo>, ApiError>> {
        Box::pin(async move {
            let accounts = self.ledger.accounts().await.map_err(|e| failed(&e))?;
            Ok(accounts.providers.iter().map(info).collect())
        })
    }

    fn remove(&self, provider: ProviderId) -> BoxFuture<'_, Result<Models, ApiError>> {
        Box::pin(async move {
            let removed = self.ledger.remove_provider(provider).await;
            if !removed.map_err(|e| failed(&e))? {
                return Err(ApiError::new(
                    code::NO_PROVIDER,
                    format!("no model account {provider} here"),
                ));
            }
            self.rebuilt().await
        })
    }
}

/// Where the API of `kind` is: the service's own, or `url` for another server.
fn base_url(kind: ProviderKind, url: Option<&str>) -> Option<String> {
    Some(
        match kind {
            ProviderKind::Openai => "https://api.openai.com/v1",
            ProviderKind::Anthropic => "https://api.anthropic.com",
            ProviderKind::Openrouter => "https://openrouter.ai/api/v1",
            ProviderKind::Compatible => url?,
        }
        .to_owned(),
    )
}

/// The endpoint of `model` at `account`, with its key.
pub fn endpoint_of(account: &ProviderConnected, model: &str) -> Option<Endpoint> {
    let base = base_url(account.kind, account.url.as_deref())?;
    let mut provider = Config::for_url(base, Some(account.key.expose().to_owned()))?;
    if model::speaks_anthropic(&provider) {
        "anthropic".clone_into(&mut provider.provider);
    }
    Some(Endpoint {
        local: provider.is_local(),
        provider,
        claude: None,
        model: model.to_owned(),
        gateway: None,
        at: AT.to_owned(),
        key_from: KEY_FROM,
    })
}

/// The account as a client may see it: never its key.
fn info(account: &ProviderConnected) -> ProviderInfo {
    ProviderInfo {
        id: account.provider,
        kind: account.kind,
        url: account.url.clone(),
        hint: hint(account.key.expose()),
        models: account.models.clone(),
    }
}

/// The key as pasted, without the spaces around it: one line of visible characters.
fn checked_key(key: &Secret) -> Result<Secret, ApiError> {
    let key = key.expose().trim();
    if key.is_empty()
        || key.len() > LONGEST_KEY
        || key.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(invalid(
            "that does not look like a key: paste the whole key, and nothing else",
        ));
    }
    Ok(Secret::new(key))
}

/// The URL another server takes, `https` or `http` on this computer, without credentials in
/// it; none for a service whose API glave knows.
fn checked_url(kind: ProviderKind, url: Option<String>) -> Result<Option<String>, ApiError> {
    let url = url.map(|url| url.trim().trim_end_matches('/').to_owned());
    match (kind, url) {
        (ProviderKind::Compatible, None) => Err(invalid(
            "give the server's address, such as http://localhost:11434/v1",
        )),
        (ProviderKind::Compatible, Some(url)) => {
            let config = Config::for_url(url.clone(), None)
                .filter(|_| !url.contains(['@', '?', '#', ' ']))
                .ok_or_else(|| invalid(NOT_AN_ADDRESS))?;
            if url.starts_with("http://") && !config.is_local() {
                return Err(invalid(
                    "use an https address, or http only for a server on this computer",
                ));
            }
            Ok(Some(url))
        }
        (_, Some(_)) => Err(invalid("only another server takes an address")),
        (_, None) => Ok(None),
    }
}

/// Said of a server address glave cannot use.
const NOT_AN_ADDRESS: &str = "that is not a server address glave can use: give its API address, such as \
     http://localhost:11434/v1";

/// `-32602`, saying what to change.
fn invalid(message: &str) -> ApiError {
    ApiError::new(code::INVALID_PARAMS, message)
}

/// `-32020` with `failure` and `message`.
fn provider_failed(failure: Failure, message: String) -> ApiError {
    ApiError {
        code: code::PROVIDER,
        message,
        data: Some(ErrorData { failure }),
    }
}

/// What the service is, in a person's words.
fn service(account: &ProviderConnected) -> String {
    match account.kind {
        ProviderKind::Openai => "OpenAI".to_owned(),
        ProviderKind::Anthropic => "Anthropic".to_owned(),
        ProviderKind::Openrouter => "OpenRouter".to_owned(),
        ProviderKind::Compatible => format!(
            "the server at {}",
            account.url.as_deref().unwrap_or("that URL")
        ),
    }
}

/// Why the check of `account`'s key failed, and what to do.
fn refused(account: &ProviderConnected, error: &ProviderError) -> ApiError {
    let failure = error.failure();
    let service = service(account);
    let message = match failure {
        Failure::Refused => format!(
            "{service} did not accept that key: check that you copied all of it and that it is \
             still active"
        ),
        Failure::NoCredit => {
            format!("your account at {service} has no credit left: add some there, then try again")
        }
        Failure::Unreachable | Failure::Silent => match account.kind {
            ProviderKind::Compatible => format!(
                "glave could not reach {service}: check that it is running and the URL is right"
            ),
            _ => format!("glave could not reach {service}: check your internet connection"),
        },
        Failure::RateLimited | Failure::Unavailable => {
            format!("{service} is busy right now: try again in a minute")
        }
        _ if account.kind == ProviderKind::Compatible => format!(
            "{service} did not answer as glave expected: check the URL is the server's API \
             address"
        ),
        _ => format!("{service} did not answer as glave expected: try again shortly"),
    };
    provider_failed(failure, message)
}

/// A failure of glave's own, such as the store's.
fn failed(error: &anyhow::Error) -> ApiError {
    ApiError::new(code::INTERNAL, format!("{error:#}"))
}
