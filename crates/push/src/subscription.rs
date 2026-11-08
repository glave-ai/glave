//! A browser's push subscription, as `PushManager.subscribe` makes it: where its push service
//! takes pushes for it (the endpoint), and the keys a message to it is encrypted to (RFC 8291:
//! its P-256 public key and a 16-byte secret). Read once, when the browser hands it over, so
//! every one kept is one a push can go to: an endpoint of one of the browsers' own push services
//! ([`PUSH_SERVICES`]), over `https` on 443, so no subscription points a push anywhere else.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use thiserror::Error;
use url::{Host, Url};
use web_push_native::Auth;
use web_push_native::p256::PublicKey;

/// The longest endpoint taken: push services' are a few hundred characters.
const LONGEST: usize = 2_048;

/// The push services browsers subscribe with, by host: Google's for Chrome, Edge on Android,
/// Samsung Internet and Opera; Mozilla's for Firefox; Apple's for Safari; Microsoft's (WNS) for
/// Edge on Windows. A leading `*.` is any name under it.
pub const PUSH_SERVICES: [&str; 4] = [
    "fcm.googleapis.com",
    "*.push.services.mozilla.com",
    "*.push.apple.com",
    "*.notify.windows.com",
];

/// Why a subscription was not taken. It never says the endpoint, which only its browser and the
/// server should know.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Refused {
    #[error("its endpoint is not one of the browsers' push services, over https")]
    Endpoint,
    #[error("its key is not a P-256 public key in base64url")]
    Key,
    #[error("its secret is not 16 bytes in base64url")]
    Secret,
}

/// A subscription a push can go to.
#[derive(Clone)]
pub struct Subscription {
    pub(crate) endpoint: Url,
    pub(crate) key: PublicKey,
    pub(crate) secret: Auth,
}

impl fmt::Debug for Subscription {
    /// Its push service alone: the endpoint is the browser's address, the secret its own.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("service", &self.endpoint.host_str().unwrap_or_default())
            .finish_non_exhaustive()
    }
}

impl Subscription {
    /// Reads `endpoint`, `p256dh` and `auth` as a browser gives them (`PushSubscription.toJSON`):
    /// the endpoint one of [`PUSH_SERVICES`] over `https` on 443; or, when `here`, `http` to
    /// this computer, as a test's own push service is.
    pub(crate) fn read(
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        here: bool,
    ) -> Result<Self, Refused> {
        let endpoint = endpoint_of(endpoint, here).ok_or(Refused::Endpoint)?;
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
}

/// `text` as base64url, padded or not.
fn decoded(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text.trim_end_matches('=')).ok()
}

/// Whether `host` is one of [`PUSH_SERVICES`].
fn is_push_service(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    PUSH_SERVICES
        .iter()
        .any(|service| match service.strip_prefix("*.") {
            Some(domain) => host
                .strip_suffix(domain)
                .is_some_and(|name| name.len() > 1 && name.ends_with('.')),
            None => host == *service,
        })
}

/// `text` as a push service's endpoint: an `https` URL on 443 at a name of [`PUSH_SERVICES`],
/// with no user, at most [`LONGEST`] long; or, when `here`, an `http` URL to this computer.
fn endpoint_of(text: &str, here: bool) -> Option<Url> {
    if text.len() > LONGEST {
        return None;
    }
    let url = Url::parse(text).ok()?;
    let named = url.username().is_empty() && url.password().is_none();
    let fits = match (url.scheme(), url.host()) {
        ("https", Some(Host::Domain(host))) => {
            is_push_service(host) && url.port().is_none_or(|port| port == 443)
        }
        ("http", Some(host)) => here && is_local(&host),
        _ => false,
    };
    (named && fits).then_some(url)
}

/// Whether `host` is this computer.
fn is_local(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(v4) => v4.is_loopback(),
        Host::Ipv6(v6) => v6.is_loopback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_browsers_push_services_are_taken() {
        for host in [
            "fcm.googleapis.com",
            "updates.push.services.mozilla.com",
            "web.push.apple.com",
            "wns2-bl2p.notify.windows.com",
            "FCM.googleapis.com",
        ] {
            assert!(is_push_service(host), "{host}");
        }
        for host in [
            "googleapis.com",
            "evil-fcm.googleapis.com.example.net",
            "push.apple.com",
            "push.apple.com.example.net",
            "xpush.apple.com",
            "notify.windows.com",
            "example.com",
        ] {
            assert!(!is_push_service(host), "{host}");
        }
    }

    #[test]
    fn a_push_service_is_reached_only_over_https_on_its_own_port() {
        let ok = |text: &str| endpoint_of(text, false).is_some();
        assert!(ok("https://fcm.googleapis.com/fcm/send/abc"));
        assert!(ok("https://web.push.apple.com:443/QGx2v"));
        assert!(!ok("https://fcm.googleapis.com:8443/fcm/send/abc"));
        assert!(!ok("http://fcm.googleapis.com/fcm/send/abc"));
        assert!(!ok("https://user:pass@fcm.googleapis.com/x"));
        assert!(!ok("https://172.217.0.0/fcm/send/abc"));
        assert!(!ok(&format!(
            "https://fcm.googleapis.com/{}",
            "a".repeat(LONGEST)
        )));
        assert!(!ok("not a url"));
    }

    #[test]
    fn only_a_test_s_own_service_may_be_on_this_computer() {
        assert!(endpoint_of("http://127.0.0.1:4000/gone", true).is_some());
        assert!(endpoint_of("http://localhost:4000/x", true).is_some());
        assert!(endpoint_of("http://127.0.0.1:4000/gone", false).is_none());
        assert!(endpoint_of("http://10.0.0.1/x", true).is_none());
        assert!(endpoint_of("http://example.com/x", true).is_none());
    }
}
