//! A browser's push subscription, as `PushManager.subscribe` makes it: where its push service
//! takes pushes for it (the endpoint), and the keys a message to it is encrypted to (RFC 8291:
//! its P-256 public key and a 16-byte secret). Read once, when the browser hands it over, so
//! every one kept is one a push can go to.

use std::net::IpAddr;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use glave_ports::host::{is_this_computer, reachable};
use thiserror::Error;
use url::{Host, Url};
use web_push_native::Auth;
use web_push_native::p256::PublicKey;

/// The longest endpoint taken: push services' are a few hundred characters.
const LONGEST: usize = 2_048;

/// Why a subscription was not taken.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Refused {
    #[error("its endpoint is not an https URL of a push service: {0}")]
    Endpoint(String),
    #[error("its key is not a P-256 public key in base64url")]
    Key,
    #[error("its secret is not 16 bytes in base64url")]
    Secret,
}

/// A subscription a push can go to.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub(crate) endpoint: Url,
    pub(crate) key: PublicKey,
    pub(crate) secret: Auth,
}

impl Subscription {
    /// Reads `endpoint`, `p256dh` and `auth` as a browser gives them (`PushSubscription.toJSON`),
    /// the endpoint `https` and never a private address; `http` to this computer too when
    /// `here`, as a test's own push service is.
    pub(crate) fn read(
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        here: bool,
    ) -> Result<Self, Refused> {
        let endpoint = endpoint_of(endpoint, here)?;
        let key = decoded(p256dh)
            .and_then(|bytes| PublicKey::from_sec1_bytes(&bytes).ok())
            .ok_or(Refused::Key)?;
        let secret: [u8; 16] = decoded(auth)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(Refused::Secret)?;
        Ok(Self {
            endpoint,
            key,
            secret: Auth::from(secret),
        })
    }

    /// Where its push service takes pushes for it.
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }
}

/// `text` as base64url, padded or not.
fn decoded(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// `text` as a push service's endpoint: an `https` URL with a host and no user, at most
/// [`LONGEST`] long, whose host, when an address, is a public one; or, when `here`, an `http`
/// URL to this computer.
fn endpoint_of(text: &str, here: bool) -> Result<Url, Refused> {
    let refused = || Refused::Endpoint(text.chars().take(80).collect());
    if text.len() > LONGEST {
        return Err(refused());
    }
    let url = Url::parse(text).map_err(|_| refused())?;
    let host = url.host_str().ok_or_else(refused)?;
    let local = here && is_this_computer(host);
    let fits = match url.scheme() {
        "https" => true,
        "http" => local,
        _ => false,
    };
    let address = match url.host() {
        Some(Host::Ipv4(v4)) => Some(IpAddr::V4(v4)),
        Some(Host::Ipv6(v6)) => Some(IpAddr::V6(v6)),
        _ => None,
    };
    let public = address.is_none_or(|ip| reachable(ip, local));
    let named = url.username().is_empty() && url.password().is_none();
    (fits && public && named && (local || !is_this_computer(host)))
        .then_some(url)
        .ok_or_else(refused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::example::{AUTH, P256DH};

    fn read(endpoint: &str, here: bool) -> Result<Subscription, Refused> {
        Subscription::read(endpoint, P256DH, AUTH, here)
    }

    #[test]
    fn a_push_service_s_endpoint_is_taken() {
        let fcm = "https://fcm.googleapis.com/fcm/send/abc:def";
        assert_eq!(read(fcm, false).unwrap().endpoint(), fcm);
        let apple = "https://web.push.apple.com/QGx2v";
        assert!(read(apple, false).is_ok());
        // Padded, as some browsers write it.
        let padded = Subscription::read(apple, P256DH, &format!("{AUTH}=="), false);
        assert!(padded.is_ok());
    }

    #[test]
    fn an_endpoint_that_could_reach_a_private_network_is_refused() {
        for endpoint in [
            "http://push.example.net/x",
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://10.0.0.1/x",
            "https://[::1]/x",
            "https://169.254.169.254/latest",
            "https://user:pass@push.example.net/x",
            "ftp://push.example.net/x",
            "not a url",
        ] {
            assert!(read(endpoint, false).is_err(), "{endpoint} is refused");
        }
        let long = format!("https://push.example.net/{}", "a".repeat(LONGEST));
        assert!(read(&long, false).is_err());
    }

    #[test]
    fn only_a_test_s_own_service_may_be_on_this_computer() {
        assert!(read("http://127.0.0.1:4000/gone", true).is_ok());
        assert!(read("http://localhost:4000/x", true).is_ok());
        assert!(read("http://push.example.net/x", true).is_err());
        assert!(read("https://10.0.0.1/x", true).is_err());
        assert!(read("https://169.254.169.254/x", true).is_err());
    }

    #[test]
    fn keys_of_the_wrong_shape_are_refused() {
        let endpoint = "https://push.example.net/x";
        let half: String = P256DH.chars().take(40).collect();
        let short = Subscription::read(endpoint, &half, AUTH, false);
        assert_eq!(short.err(), Some(Refused::Key));
        let secret = Subscription::read(endpoint, P256DH, "AAAA", false);
        assert_eq!(secret.err(), Some(Refused::Secret));
        let garbled = Subscription::read(endpoint, "!!", AUTH, false);
        assert_eq!(garbled.err(), Some(Refused::Key));
    }
}
