//! One certificate ordered from an ACME authority (RFC 8555): an order for the name, its
//! authorization proved with TLS-ALPN-01 (RFC 8737) by the resolver that serves HTTPS, then a
//! new key and its request, and the chain the authority signs.

use std::time::Duration;

use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, NewOrder, OrderStatus, RetryPolicy,
};

use crate::https::HttpsError;
use crate::resolver::{Resolver, challenge_key};

/// How long the authority may take to check the name, and then to sign.
const PATIENCE: Duration = Duration::from_mins(2);

/// A certificate's chain and its private key, as PEM.
pub(crate) struct Ordered {
    pub(crate) chain: String,
    pub(crate) key: String,
}

/// Orders a certificate for `name` with `account`, answering the challenge through
/// `resolver`.
pub(crate) async fn order(
    account: &Account,
    name: &str,
    resolver: &Resolver,
) -> Result<Ordered, HttpsError> {
    let identifiers = [Identifier::Dns(name.to_owned())];
    let mut order = account
        .new_order(&NewOrder::new(&identifiers))
        .await
        .map_err(HttpsError::from)?;
    let proved = prove(&mut order, name, resolver).await;
    // Whatever the outcome, the challenge is over.
    let ready = match proved {
        Ok(()) => {
            let patience = RetryPolicy::new().timeout(PATIENCE);
            order.poll_ready(&patience).await.map_err(HttpsError::from)
        }
        Err(error) => Err(error),
    };
    resolver.answered();
    if ready? != OrderStatus::Ready {
        return Err(HttpsError::Authority(format!(
            "it did not accept that this server answers for {name}: does the name point at \
             this server, and can the internet reach the port glave serves HTTPS on?"
        )));
    }
    let key = order.finalize().await.map_err(HttpsError::from)?;
    let patience = RetryPolicy::new().timeout(PATIENCE);
    let chain = order
        .poll_certificate(&patience)
        .await
        .map_err(HttpsError::from)?;
    Ok(Ordered { chain, key })
}

/// Sets up the TLS-ALPN-01 challenge of each authorization `order` still needs, and tells the
/// authority it is ready.
async fn prove(
    order: &mut instant_acme::Order,
    name: &str,
    resolver: &Resolver,
) -> Result<(), HttpsError> {
    let mut authorizations = order.authorizations();
    while let Some(authorization) = authorizations.next().await {
        let mut authorization = authorization.map_err(HttpsError::from)?;
        match authorization.status {
            AuthorizationStatus::Pending => {}
            AuthorizationStatus::Valid => continue,
            other => {
                return Err(HttpsError::Authority(format!(
                    "its authorization for {name} is {other:?}"
                )));
            }
        }
        let mut challenge = authorization
            .challenge(ChallengeType::TlsAlpn01)
            .ok_or_else(|| {
                HttpsError::Authority(format!(
                    "it offers no TLS-ALPN-01 challenge for {name}, the one glave answers"
                ))
            })?;
        let digest = challenge.key_authorization().digest();
        resolver.challenge(challenge_key(name, digest.as_ref())?);
        challenge.set_ready().await.map_err(HttpsError::from)?;
    }
    Ok(())
}
