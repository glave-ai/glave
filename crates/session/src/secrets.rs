//! Secrets (`specs/model.md`, "Secrets"): the person on this computer adds one, such as an API
//! key, from a client, by name and with the hosts its value may reach. glave keeps it sealed in
//! their accounts ledger under a key of its own; commands see only its placeholder, and its
//! value is masked wherever output shows it ([`KnownSecrets`]). A client lists names and hosts,
//! never a value; nothing the agent calls reaches these methods.

use glave_event::Secret;
use glave_ports::auth::{REDACTED, SHORTEST_SECRET};
use glave_ports::computer::is_variable_name;
use glave_ports::secrets::{Known, KnownSecrets};
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};

use crate::accounts::{AccountLedger, AccountsError};
use crate::protocol::{ApiError, Reply, SecretInfo, code};
use crate::providers::theirs;
use crate::server::Server;
use crate::words::Sentence;

/// The longest name, host, label of a host or value taken, in bytes.
const LONGEST_NAME: usize = 128;
const LONGEST_HOST: usize = 253;
const LONGEST_LABEL: usize = 63;
const LONGEST_VALUE: usize = 16 << 10;

/// The most hosts one secret may reach.
const MOST_HOSTS: usize = 32;

/// The person's secrets: kept in their accounts ledger, and known to their workspaces.
pub struct Secrets<L> {
    ledger: AccountLedger<L>,
    known: KnownSecrets,
    /// Held while the secrets are read and made known, so the last made known is the latest.
    reading: tokio::sync::Mutex<()>,
}

impl<L: EventLog> Secrets<L> {
    /// The secrets `ledger` keeps, made known through `known` now and each time they are read.
    pub async fn open(
        ledger: AccountLedger<L>,
        known: KnownSecrets,
    ) -> Result<Self, AccountsError> {
        let reading = tokio::sync::Mutex::default();
        let secrets = Self {
            ledger,
            known,
            reading,
        };
        secrets.kept().await?;
        Ok(secrets)
    }

    /// The secrets kept now, once any removed are erased, made known to the workspaces from
    /// now on: those another process changed too.
    pub(crate) async fn kept(&self) -> Result<Vec<SecretInfo>, AccountsError> {
        let _reading = self.reading.lock().await;
        let secrets = self.ledger.accounts().await?.secrets;
        let known = secrets
            .iter()
            .map(|kept| Known::new(kept.name.clone(), kept.value.clone()));
        self.known.set(known.collect());
        Ok(secrets
            .into_iter()
            .map(|kept| SecretInfo {
                name: kept.name,
                hosts: kept.hosts,
            })
            .collect())
    }

    /// The secret called `name` as a client sees it, once kept.
    async fn shown(&self, name: &str) -> Result<SecretInfo, ApiError> {
        let kept = self.kept().await?;
        kept.into_iter()
            .find(|kept| kept.name == name)
            .ok_or_else(|| missing(name))
    }
}

/// `name` as a secret's: a variable's name.
fn named(name: &str) -> Result<&str, ApiError> {
    let name = name.trim();
    if name.len() > LONGEST_NAME || !is_variable_name(name) {
        return Err(ApiError::new(
            code::INVALID_PARAMS,
            Sentence::NotASecretName,
        ));
    }
    Ok(name)
}

/// `hosts` as a secret keeps them: each a host's name in lower case, or `*.` and a name of two
/// labels or more (never all of `*.com`), once each; at least one.
fn hosted(hosts: &[String]) -> Result<Vec<String>, ApiError> {
    let mut kept: Vec<String> = Vec::with_capacity(hosts.len());
    for host in hosts {
        let host = host.trim().to_ascii_lowercase();
        let (wildcard, name) = match host.strip_prefix("*.") {
            Some(name) => (true, name),
            None => (false, host.as_str()),
        };
        let labels = name.split('.').all(|label| {
            (1..=LONGEST_LABEL).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
        let broad = wildcard && !name.contains('.');
        if !labels || broad || host.len() > LONGEST_HOST {
            return Err(ApiError::new(code::INVALID_PARAMS, Sentence::SecretHosts));
        }
        if !kept.contains(&host) {
            kept.push(host);
        }
    }
    if kept.is_empty() || kept.len() > MOST_HOSTS {
        return Err(ApiError::new(code::INVALID_PARAMS, Sentence::SecretHosts));
    }
    Ok(kept)
}

/// `value` without the space around it, as pasted with a line's end, when it is one a secret
/// can be: one line, long enough to mask wherever it shows, not more than a key ever is, and
/// no part of what it is masked with.
fn valued(value: &Secret) -> Result<Secret, ApiError> {
    let value = value.expose().trim();
    let fits = (SHORTEST_SECRET..=LONGEST_VALUE).contains(&value.len());
    if !fits || value.chars().any(char::is_control) || REDACTED.contains(value) {
        return Err(ApiError::new(code::INVALID_PARAMS, Sentence::SecretValue));
    }
    Ok(Secret::new(value))
}

/// No secret called `name` is kept.
fn missing(name: &str) -> ApiError {
    let name = name.to_owned();
    ApiError::new(code::NO_CREDENTIAL, Sentence::NoSuchSecret { name })
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The person's secrets, for the person on this computer only.
    fn secrets_for(&self, caller: &Caller) -> Result<&Secrets<L>, ApiError> {
        let secrets = self.inner.secrets.as_deref();
        theirs(
            caller,
            secrets,
            Sentence::SecretsNotHere,
            Sentence::SecretsOnItsComputer,
        )
    }

    /// Answers `secret/list`.
    pub(crate) async fn list_secrets(&self, caller: &Caller) -> Result<Reply, ApiError> {
        let secrets = self.secrets_for(caller)?.kept().await?;
        Ok(Reply::Secrets { secrets })
    }

    /// Answers `secret/add`.
    pub(crate) async fn add_secret(
        &self,
        caller: &Caller,
        name: &str,
        hosts: &[String],
        value: &Secret,
    ) -> Result<Reply, ApiError> {
        let secrets = self.secrets_for(caller)?;
        let (name, hosts, value) = (named(name)?, hosted(hosts)?, valued(value)?);
        if !secrets.ledger.add_secret(name, &hosts, &value).await? {
            let name = name.to_owned();
            let kept = Sentence::SecretKept { name };
            return Err(ApiError::new(code::INVALID_PARAMS, kept));
        }
        let secret = secrets.shown(name).await?;
        Ok(Reply::Secret { secret })
    }

    /// Answers `secret/replace`.
    pub(crate) async fn replace_secret(
        &self,
        caller: &Caller,
        name: &str,
        hosts: Option<&[String]>,
        value: &Secret,
    ) -> Result<Reply, ApiError> {
        let secrets = self.secrets_for(caller)?;
        let (name, value) = (named(name)?, valued(value)?);
        let hosts = hosts.map(hosted).transpose()?;
        let ledger = &secrets.ledger;
        if !ledger
            .replace_secret(name, hosts.as_deref(), &value)
            .await?
        {
            return Err(missing(name));
        }
        let secret = secrets.shown(name).await?;
        Ok(Reply::Secret { secret })
    }

    /// Answers `secret/remove`.
    pub(crate) async fn remove_secret(
        &self,
        caller: &Caller,
        name: &str,
    ) -> Result<Reply, ApiError> {
        let secrets = self.secrets_for(caller)?;
        let name = named(name)?;
        if !secrets.ledger.remove_secret(name).await? {
            return Err(missing(name));
        }
        secrets.kept().await?;
        Ok(Reply::Done {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_hosts_and_values_are_checked() {
        assert_eq!(named(" STRIPE_KEY ").unwrap(), "STRIPE_KEY");
        for wrong in ["", "1KEY", "MY-KEY", "A B", &"K".repeat(129)] {
            assert!(named(wrong).is_err(), "{wrong:?}");
        }
        let hosts = [
            "API.Stripe.com".into(),
            "*.stripe.com".into(),
            "api.stripe.com".into(),
        ];
        assert_eq!(hosted(&hosts).unwrap(), ["api.stripe.com", "*.stripe.com"]);
        for wrong in [
            vec![],
            vec!["https://api.stripe.com".into()],
            vec!["api.stripe.com/v1".into()],
            vec!["*".into()],
            vec!["a..b".into()],
            vec!["-a.com".into()],
            vec!["api.stripe.com:443".into()],
            vec!["*.com".into()],
            vec![format!("{}.com", "a".repeat(64))],
        ] {
            assert!(hosted(&wrong).is_err(), "{wrong:?}");
        }
        let value = glave_ports::testing::any_secret();
        let pasted = valued(&Secret::new(format!(" {value}\n"))).unwrap();
        assert_eq!(pasted.expose(), value, "kept without the space around it");
        assert!(valued(&Secret::new("short")).is_err());
        assert!(valued(&Secret::new(format!("{value}\tmore"))).is_err());
        assert!(valued(&Secret::new("redacted")).is_err());
        assert!(valued(&Secret::new("x".repeat(LONGEST_VALUE + 1))).is_err());
    }
}
