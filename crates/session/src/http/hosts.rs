//! `Host` and `Origin` on the person's loopback listener (`specs/one-server.md`, "Finding
//! it"), a second line behind the sign-in: a request whose `Host` is not the listener's own
//! address (or `localhost` at its port) is refused, so a name rebound to 127.0.0.1 reads
//! nothing there, not even the app's files; so is one whose `Origin` is not the server's own,
//! so a page elsewhere calls nothing. A listener on other addresses sits behind a proxy whose
//! names it cannot know, and checks neither.

use std::net::SocketAddr;

use glave_ports::host::is_loopback;
use hyper::header::{self, HeaderMap};

/// The names a loopback listener answers to.
pub(crate) struct ListenerNames {
    names: [String; 2],
}

impl ListenerNames {
    /// The names of the listener bound at `bound`, when it listens on loopback alone.
    pub(crate) fn of(bound: SocketAddr) -> Option<Self> {
        is_loopback(bound.ip()).then(|| Self {
            names: [bound.to_string(), format!("localhost:{}", bound.port())],
        })
    }

    /// Whether a request with `headers` came to one of these names, from a page of its own if
    /// from a page at all.
    pub(crate) fn allow(&self, headers: &HeaderMap) -> bool {
        let Some(host) = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        if !self
            .names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(host))
        {
            return false;
        }
        headers.get(header::ORIGIN).is_none_or(|origin| {
            origin
                .to_str()
                .ok()
                .and_then(|origin| origin.strip_prefix("http://"))
                .is_some_and(|from| from.eq_ignore_ascii_case(host))
        })
    }
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;

    use super::*;

    fn headers(host: Option<&str>, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in [(header::HOST, host), (header::ORIGIN, origin)] {
            if let Some(value) = value {
                headers.insert(name, HeaderValue::from_str(value).unwrap());
            }
        }
        headers
    }

    #[test]
    fn a_loopback_listener_answers_only_its_own_names_and_pages() {
        let hosts = ListenerNames::of("127.0.0.1:4180".parse().unwrap()).unwrap();
        for (host, origin) in [
            (Some("127.0.0.1:4180"), None),
            (Some("localhost:4180"), None),
            (Some("127.0.0.1:4180"), Some("http://127.0.0.1:4180")),
            (Some("LOCALHOST:4180"), Some("http://localhost:4180")),
        ] {
            assert!(hosts.allow(&headers(host, origin)), "{host:?} {origin:?}");
        }
        for (host, origin) in [
            (None, None),
            (Some("rebound.example.com:4180"), None),
            (Some("127.0.0.1:4181"), None),
            (Some("127.0.0.1:4180"), Some("http://evil.example.com")),
            (Some("127.0.0.1:4180"), Some("null")),
            (Some("127.0.0.1:4180"), Some("http://localhost:4180")),
            (Some("127.0.0.1:4180"), Some("https://127.0.0.1:4180")),
        ] {
            assert!(!hosts.allow(&headers(host, origin)), "{host:?} {origin:?}");
        }
    }

    #[test]
    fn only_a_loopback_listener_checks() {
        assert!(ListenerNames::of("0.0.0.0:8080".parse().unwrap()).is_none());
        assert!(ListenerNames::of("192.168.1.2:8080".parse().unwrap()).is_none());
        let ipv6 = ListenerNames::of("[::1]:4180".parse().unwrap()).unwrap();
        assert!(ipv6.allow(&headers(Some("[::1]:4180"), Some("http://[::1]:4180"))));
    }
}
