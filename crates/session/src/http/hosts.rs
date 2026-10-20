//! `Host` and `Origin` (`specs/one-server.md`, "Finding it"), a second line behind the
//! sign-in. On a loopback listener, a request whose `Host` is not the listener's own address
//! (or `localhost` at its port, or the server's public origin, which a tunnel on this computer
//! forwards) is refused, so a name rebound to 127.0.0.1 reads nothing there, not even the app's
//! files; so is one from a page other than the server's own: at the name the request came to,
//! or the public origin. A listener on other addresses sits behind a proxy whose names it
//! cannot know, so it checks no `Host`; with a public origin it refuses every other page, and
//! without one it checks nothing.

use std::net::SocketAddr;

use glave_ports::host::{PublicOrigin, is_loopback};
use hyper::header::{self, HeaderMap};

/// What a listener answers to.
pub(crate) enum ListenerNames {
    /// A loopback listener: its own names, and the public origin a tunnel here forwards.
    Loopback {
        names: [String; 2],
        origin: Option<PublicOrigin>,
    },
    /// A listener beyond this computer, behind what serves HTTPS for this origin.
    Public(PublicOrigin),
}

impl ListenerNames {
    /// The names of the listener bound at `bound`, reached from elsewhere at `origin` if
    /// anywhere; none for a listener beyond this computer with no origin, which checks nothing.
    pub(crate) fn of(bound: SocketAddr, origin: Option<&PublicOrigin>) -> Option<Self> {
        if is_loopback(bound.ip()) {
            return Some(Self::Loopback {
                names: [bound.to_string(), format!("localhost:{}", bound.port())],
                origin: origin.cloned(),
            });
        }
        origin.cloned().map(Self::Public)
    }

    /// Whether a request with `headers` came to one of these names, from a page of the
    /// server's own if from a page at all.
    pub(crate) fn allow(&self, headers: &HeaderMap) -> bool {
        let named = |name: header::HeaderName| {
            headers
                .get(name)
                .map(|value| value.to_str().ok().unwrap_or_default())
        };
        let page = named(header::ORIGIN);
        match self {
            Self::Public(origin) => {
                page.is_none_or(|page| page.eq_ignore_ascii_case(origin.as_str()))
            }
            Self::Loopback { names, origin } => {
                let Some(host) = named(header::HOST) else {
                    return false;
                };
                let own = names.iter().any(|name| name.eq_ignore_ascii_case(host));
                let public = origin
                    .as_ref()
                    .filter(|origin| origin.authority().eq_ignore_ascii_case(host));
                if !own && public.is_none() {
                    return false;
                }
                page.is_none_or(|page| {
                    let at_its_name = own
                        && page
                            .strip_prefix("http://")
                            .is_some_and(|from| from.eq_ignore_ascii_case(host));
                    at_its_name
                        || origin
                            .as_ref()
                            .is_some_and(|origin| page.eq_ignore_ascii_case(origin.as_str()))
                })
            }
        }
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
        let hosts = ListenerNames::of("127.0.0.1:4180".parse().unwrap(), None).unwrap();
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
            (Some("glave.example.com"), None),
            (Some("glave.example.com"), Some("https://glave.example.com")),
        ] {
            assert!(!hosts.allow(&headers(host, origin)), "{host:?} {origin:?}");
        }
    }

    #[test]
    fn a_tunnel_here_reaches_a_loopback_listener_at_the_public_origin_alone() {
        let origin = PublicOrigin::parse("https://glave.example.com").unwrap();
        let hosts = ListenerNames::of("127.0.0.1:4180".parse().unwrap(), Some(&origin)).unwrap();
        for (host, page) in [
            (Some("glave.example.com"), None),
            (Some("Glave.Example.com"), Some("https://glave.example.com")),
            (Some("127.0.0.1:4180"), Some("http://127.0.0.1:4180")),
            // A proxy that names its upstream in `Host`, for a page at the public origin.
            (Some("127.0.0.1:4180"), Some("https://glave.example.com")),
        ] {
            assert!(hosts.allow(&headers(host, page)), "{host:?} {page:?}");
        }
        for (host, page) in [
            (Some("glave.example.com"), Some("http://glave.example.com")),
            (Some("glave.example.com"), Some("https://evil.example.com")),
            (Some("glave.example.com:8443"), None),
            (Some("other.example.com"), Some("https://glave.example.com")),
        ] {
            assert!(!hosts.allow(&headers(host, page)), "{host:?} {page:?}");
        }
    }

    #[test]
    fn a_listener_beyond_this_computer_takes_pages_of_its_origin_alone() {
        let any: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        assert!(
            ListenerNames::of(any, None).is_none(),
            "nothing to check by"
        );
        assert!(ListenerNames::of("192.168.1.2:8080".parse().unwrap(), None).is_none());
        let origin = PublicOrigin::parse("https://glave.example.com").unwrap();
        let hosts = ListenerNames::of(any, Some(&origin)).unwrap();
        // A proxy's own checks name the address it reaches glave at.
        assert!(hosts.allow(&headers(Some("10.0.0.5:8080"), None)));
        assert!(hosts.allow(&headers(None, Some("https://glave.example.com"))));
        assert!(!hosts.allow(&headers(None, Some("https://evil.example.com"))));
        assert!(!hosts.allow(&headers(None, Some("http://10.0.0.5:8080"))));
    }

    #[test]
    fn ipv6_loopback_has_its_names_too() {
        let ipv6 = ListenerNames::of("[::1]:4180".parse().unwrap(), None).unwrap();
        assert!(ipv6.allow(&headers(Some("[::1]:4180"), Some("http://[::1]:4180"))));
    }
}
