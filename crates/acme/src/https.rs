//! HTTPS for one name: the certificate served, kept current. A certificate kept in the store is
//! served the moment the server opens; a new one is ordered when there is none, and again with
//! a third of its life left, and an order that fails is tried again later, waiting longer each
//! time, so an authority's rate limits are never run into.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use glave_event::{AcmeRegistered, At, CertificateIssued, Secret};
use glave_ports::{Clock, Entropy, EntropyError, EventLog, Scope};
use instant_acme::{Account, AccountCredentials, NewAccount};
use jiff::{SignedDuration, Timestamp};
use rustls::ServerConfig;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use thiserror::Error;

use crate::kept::Ledger;
use crate::order::order;
use crate::resolver::{Resolver, certified, config};

/// The longest glave sleeps before it looks again whether its certificate is due, so a
/// computer that slept, or a clock that jumped, never keeps an old certificate long.
const LOOK_AGAIN: Duration = Duration::from_hours(24);

/// How long glave waits after its first failed order; each failure after doubles it.
const FIRST_WAIT: Duration = Duration::from_mins(5);

/// The longest glave waits between failed orders.
const LONGEST_WAIT: Duration = Duration::from_hours(6);

/// Why HTTPS could not be served or kept current. Says what failed, never a key.
#[derive(Debug, Error)]
pub enum HttpsError {
    #[error("the certificate authority failed: {0}")]
    Authority(String),
    #[error("could not keep the certificate in glave's records: {0}")]
    Keep(String),
    #[error("could not make or read a certificate: {0}")]
    Make(String),
    #[error(transparent)]
    Random(#[from] EntropyError),
}

impl From<instant_acme::Error> for HttpsError {
    fn from(error: instant_acme::Error) -> Self {
        Self::Authority(error.to_string())
    }
}

/// A certificate in hand: the name it is for, and when it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    pub name: String,
    pub not_after: Timestamp,
}

/// What became of one order, for the server to say.
#[derive(Debug)]
pub enum Attempt {
    Issued(Issued),
    /// It failed, and is tried again after `again`.
    Failed {
        error: HttpsError,
        again: Duration,
    },
}

/// HTTPS for `name`, its certificates from the ACME directory `directory`, kept in the
/// certificates ledger of a store.
pub struct Https<L> {
    ledger: Ledger<L>,
    name: String,
    directory: String,
    resolver: Arc<Resolver>,
    config: Arc<ServerConfig>,
    /// When the certificate served now began and ends.
    life: Mutex<Option<(Timestamp, Timestamp)>>,
}

impl<L: EventLog> Https<L> {
    /// HTTPS for `name`, kept in the ledger in `scope` of `log`, serving at once the certificate
    /// kept there for it, if any. Nothing is asked of the authority until [`Https::renew`].
    pub async fn open(
        log: L,
        scope: Scope,
        name: &str,
        directory: &str,
        clock: Box<dyn Clock>,
        entropy: Box<dyn Entropy>,
    ) -> Result<Self, HttpsError> {
        let ledger = Ledger {
            log,
            scope,
            clock,
            entropy,
        };
        let resolver = Arc::new(Resolver::new(name.to_owned()));
        let mut life = None;
        if let Some(kept) = ledger.read(name, directory).await?.certificate {
            resolver.serve(certified(&kept.chain, kept.key.expose())?);
            life = Some((kept.not_before.timestamp(), kept.not_after.timestamp()));
        }
        Ok(Self {
            ledger,
            name: name.to_owned(),
            directory: directory.to_owned(),
            config: config(Arc::clone(&resolver))?,
            resolver,
            life: Mutex::new(life),
        })
    }

    /// What to serve HTTPS with: the certificate in hand, and the authority's challenge.
    pub fn tls(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.config)
    }

    /// Whether a certificate is in hand, so HTTPS is served.
    pub fn ready(&self) -> bool {
        self.resolver.ready()
    }

    /// When a new certificate is due: with a third of the one in hand's life left, or now.
    pub fn due(&self) -> Option<Timestamp> {
        let life = *self.life.lock().unwrap_or_else(PoisonError::into_inner);
        life.map(|(began, ends)| due(began, ends))
    }

    /// Orders a certificate now, keeps it, and serves it from the next handshake on.
    pub async fn renew(&self) -> Result<Issued, HttpsError> {
        let account = self.account().await?;
        let ordered = order(&account, &self.name, &self.resolver).await?;
        let key = certified(&ordered.chain, &ordered.key)?;
        let (began, ends) = life(&ordered.chain)?;
        self.ledger
            .issued(&CertificateIssued {
                name: self.name.clone(),
                chain: ordered.chain,
                key: Secret::new(ordered.key),
                not_before: At::from(began),
                not_after: At::from(ends),
            })
            .await?;
        self.resolver.serve(key);
        *self.life.lock().unwrap_or_else(PoisonError::into_inner) = Some((began, ends));
        Ok(Issued {
            name: self.name.clone(),
            not_after: ends,
        })
    }

    /// Keeps a certificate in hand until dropped: one ordered at once when there is none, and
    /// a new one whenever it is due; `told` hears of each order.
    #[expect(
        clippy::infinite_loop,
        reason = "it runs until dropped; an async fn cannot return `!` on stable Rust"
    )]
    pub async fn keep(&self, told: impl Fn(&Attempt)) {
        let mut failures = 0u32;
        loop {
            let now = self.ledger.clock.now();
            let wait = if failures > 0 {
                backoff(failures)
            } else {
                until(self.due(), now)
            };
            tokio::time::sleep(wait).await;
            let now = self.ledger.clock.now();
            if failures == 0 && self.due().is_some_and(|due| now < due) {
                continue;
            }
            match self.renew().await {
                Ok(issued) => {
                    failures = 0;
                    told(&Attempt::Issued(issued));
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    let again = backoff(failures);
                    told(&Attempt::Failed { error, again });
                }
            }
        }
    }

    /// The account kept for the directory, or one registered there now and kept.
    async fn account(&self) -> Result<Account, HttpsError> {
        let builder = || Account::builder().map_err(HttpsError::from);
        let kept = self.ledger.read(&self.name, &self.directory).await?;
        if let Some(account) = kept.account {
            let credentials: AccountCredentials =
                serde_json::from_str(account.credentials.expose()).map_err(|error| {
                    HttpsError::Keep(format!("the account kept is not readable: {error}"))
                })?;
            return builder()?
                .from_credentials(credentials)
                .await
                .map_err(HttpsError::from);
        }
        let new = NewAccount {
            contact: &[],
            terms_of_service_agreed: true,
            only_return_existing: false,
        };
        let (account, credentials) = builder()?
            .create(&new, self.directory.clone(), None)
            .await
            .map_err(HttpsError::from)?;
        let credentials = serde_json::to_string(&credentials)
            .map_err(|error| HttpsError::Keep(error.to_string()))?;
        self.ledger
            .registered(&AcmeRegistered {
                directory: self.directory.clone(),
                credentials: Secret::new(credentials),
            })
            .await?;
        Ok(account)
    }
}

/// When a certificate that began at `began` and ends at `ends` is due for renewal: with a third
/// of its life left, as Let's Encrypt advises.
fn due(began: Timestamp, ends: Timestamp) -> Timestamp {
    let third = ends.duration_since(began) / 3;
    ends.checked_sub(third).unwrap_or(ends)
}

/// How long to sleep from `now` until `due`, at most [`LOOK_AGAIN`]: none when it is due
/// already or there is no certificate.
fn until(due: Option<Timestamp>, now: Timestamp) -> Duration {
    let Some(due) = due else {
        return Duration::ZERO;
    };
    let left = due.duration_since(now).max(SignedDuration::ZERO);
    Duration::try_from(left)
        .unwrap_or(Duration::ZERO)
        .min(LOOK_AGAIN)
}

/// How long to wait after `failures` failed orders in a row: [`FIRST_WAIT`], doubling, at most
/// [`LONGEST_WAIT`].
fn backoff(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    FIRST_WAIT
        .saturating_mul(2u32.saturating_pow(doublings))
        .min(LONGEST_WAIT)
}

/// When the first certificate of `chain`, as PEM, begins and ends.
fn life(chain: &str) -> Result<(Timestamp, Timestamp), HttpsError> {
    let unread = |what: &dyn std::fmt::Display| HttpsError::Make(what.to_string());
    let leaf = CertificateDer::pem_slice_iter(chain.as_bytes())
        .next()
        .ok_or_else(|| HttpsError::Make("the authority sent no certificate".into()))?
        .map_err(|error| unread(&error))?;
    let (_, parsed) =
        x509_parser::parse_x509_certificate(leaf.as_ref()).map_err(|error| unread(&error))?;
    let validity = parsed.validity();
    let at = |seconds: i64| Timestamp::from_second(seconds).map_err(|error| unread(&error));
    Ok((
        at(validity.not_before.timestamp())?,
        at(validity.not_after.timestamp())?,
    ))
}

#[cfg(test)]
mod tests;
