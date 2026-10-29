//! A connection's token for each request to its server: read from the ledger, renewed first when it
//! expires within five minutes (`credentials/renewing.rs`), and sent as a bearer token to the
//! server at that address alone. A token past its time is never sent, so the server asks the
//! person to sign in again; a refresh token its authorization server refused is dropped, so
//! the sign-in reads as expired once its token's time is up.

use std::sync::Arc;
use std::time::Instant;

use glave_event::{ConnectionConnected, Secret};
use glave_mcp::{SignInError, address, renew};
use glave_ports::auth::{Auth, AuthError, Header, Outgoing, RequestSigner};
use glave_ports::{BoxFuture, Clock as _, SystemClock};
use glave_session::AccountsError;

use super::Connected;
use super::flows::expires;
use crate::accounts::Ledger;
use crate::credentials::new_id;
use crate::credentials::renewing::{Ended, RENEW_BEFORE, Renew, Renewing};

impl Connected {
    /// What signs each request to the person's server at `url` in: their sign-in there, as it
    /// is when the request goes out.
    pub(crate) fn signer(self: &Arc<Self>, url: &str) -> Auth {
        match address(url) {
            Some(url) => Auth::Changing(Arc::new(Signer {
                connected: Arc::clone(self),
                url,
            })),
            None => Auth::None,
        }
    }

    /// The sign-in kept for the server at `url`, if one is.
    pub(super) async fn kept_for(&self, url: &str) -> Option<ConnectionConnected> {
        match self.kept().await {
            Ok(kept) => kept.into_iter().find(|kept| kept.url == url),
            Err(error) => {
                tracing::warn!("could not read the sign-in to {url}: {}", error.message);
                None
            }
        }
    }

    /// The token for a request to the server at `url`, renewed first when it is about to
    /// expire; none without a sign-in that still works.
    pub(super) async fn token(&self, url: &str) -> Option<Secret> {
        let mut current = self.kept_for(url).await?;
        if left(&current).is_some_and(|left| left < RENEW_BEFORE) && current.refresh.is_some() {
            let renewing = |fresh| {
                let job = ConnectionRenewal {
                    ledger: self.ledger.clone(),
                    current: current.clone(),
                };
                Some(Renewing { job, fresh })
            };
            let unkept = self.renewals.wait(current.connection, renewing).await;
            let kept = self.kept_for(url).await?;
            // A renewal glave could not keep serves while the ledger holds what it renewed.
            current = match unkept {
                Some(fresh) if kept.connection == current.connection => fresh,
                _ => kept,
            };
        }
        left(&current)
            .is_none_or(|left| left > 0)
            .then_some(current.token)
    }
}

/// Seconds until `connection`'s token expires, for one that does.
fn left(connection: &ConnectionConnected) -> Option<i64> {
    let expires = connection.expires.as_ref()?.timestamp().as_second();
    Some(expires.saturating_sub(SystemClock.now().as_second()))
}

/// Signs requests to one server in with the person's sign-in there.
struct Signer {
    connected: Arc<Connected>,
    url: String,
}

impl RequestSigner for Signer {
    fn authorize<'a>(
        &'a self,
        _request: &'a Outgoing<'a>,
    ) -> BoxFuture<'a, Result<Vec<Header>, AuthError>> {
        Box::pin(async move {
            let token = self.connected.token(&self.url).await;
            Ok(token
                .map(|token| Header {
                    name: "authorization".to_owned(),
                    value: Secret::new(format!("Bearer {}", token.expose())),
                })
                .into_iter()
                .collect())
        })
    }

    fn describe(&self) -> String {
        format!("your sign-in to {}", self.url)
    }

    fn insufficient(&self, scope: &str) {
        self.connected.more.ask(&self.url, scope);
    }
}

/// Renewing a connection's token.
struct ConnectionRenewal {
    ledger: Ledger,
    current: ConnectionConnected,
}

impl Renew for ConnectionRenewal {
    type Kept = ConnectionConnected;

    fn url(&self) -> &str {
        &self.current.url
    }

    async fn renewed(&self) -> Result<ConnectionConnected, Ended<ConnectionConnected>> {
        let current = &self.current;
        let url = &current.url;
        let failed = || Ended::Failed(Instant::now());
        let kept = match self.ledger.accounts().await {
            Ok(accounts) => accounts
                .connections
                .iter()
                .any(|kept| kept.connection == current.connection),
            Err(error) => {
                tracing::warn!("could not read the connections to renew {url}: {error:#}");
                return Err(failed());
            }
        };
        let (true, Some(refresh)) = (kept, &current.refresh) else {
            return Err(Ended::Done(Instant::now()));
        };
        // Made before the refresh token is spent, so nothing after can lose what it gets.
        let id = new_id().map_err(|error| {
            tracing::warn!("could not make an id to renew {url}: {error:#}");
            failed()
        })?;
        match renew(url, &current.client, refresh).await {
            Ok(granted) => Ok(ConnectionConnected {
                connection: id,
                token: granted.token,
                expires: expires(granted.lasts),
                refresh: granted.refresh,
                client: granted.client,
                ..current.clone()
            }),
            Err(SignInError::Unreachable { message, .. }) => {
                tracing::warn!("could not renew the sign-in to {url}: {message}");
                Err(failed())
            }
            Err(refused) => {
                tracing::warn!("{refused}: it is kept without what renewed it");
                Ok(ConnectionConnected {
                    connection: id,
                    refresh: None,
                    ..current.clone()
                })
            }
        }
    }

    async fn keep(&self, fresh: &ConnectionConnected) -> Result<bool, AccountsError> {
        self.ledger
            .renew_connection(self.current.connection, fresh)
            .await
    }
}
