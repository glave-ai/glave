//! `Host` and `Origin` (`specs/one-server.md`, "Finding it"), a second line behind the
//! sign-in. On a loopback listener, a request whose `Host` is not the listener's own address
//! (or `localhost` at its port, or the server's public origin, which a tunnel on this computer
//! forwards) is refused, so a name rebound to 127.0.0.1 reads nothing there, not even the app's
//! files; so is one from a page other than the server's own: at the name the request came to,
//! or the public origin. A listener on other addresses sits behind a proxy whose names it
//! cannot know, so it checks no `Host`; with a public origin it refuses every other page, and
//! without one it checks nothing. And where a request came from, which a proxy in front of
//! glave names (`client`).

use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU8;

use glave_identity::Door;
use glave_ports::host::{PublicOrigin, is_inside, is_loopback, is_this_computer};
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

/// Which way a request with `headers` from `peer` came in, to a listener with `names` (none
/// where it checks no names): through the public origin when it came to that name, or to a
/// listener beyond this computer from elsewhere; else from this computer to the server's own
/// address when it came from here, and from elsewhere when not.
pub(crate) fn door(names: Option<&ListenerNames>, headers: &HeaderMap, peer: IpAddr) -> Door {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    match names {
        Some(ListenerNames::Loopback {
            origin: Some(origin),
            ..
        }) if origin.authority().eq_ignore_ascii_case(host) => Door::Public,
        Some(ListenerNames::Loopback { .. }) => Door::Here,
        Some(ListenerNames::Public(_)) if is_loopback(peer) && here(host) => Door::Here,
        Some(ListenerNames::Public(_)) => Door::Public,
        None if is_loopback(peer) => Door::Here,
        None => Door::Elsewhere,
    }
}

/// The network address a request with `headers` from `peer` came from, to a listener serving
/// HTTPS of its own when `secure`: one reached directly, its certificate won at its own port,
/// whose peer is the client whatever it says. Over plain HTTP, where the server's
/// configuration says how many addresses the proxies in front of it add to `X-Forwarded-For`
/// (`forwarded`), the client's first, the request came from that one, whatever any before it
/// say, and from `peer` when there are fewer, as a request that never went through them. Else
/// a peer on this computer or on a private network is a proxy: a tunnel here, or what serves
/// the origin's HTTPS, which glave listens beyond loopback only behind; the request came from
/// the last address it names, the one it took the connection from. Any other peer is the
/// client, and what it says of where it came from is not believed.
pub(crate) fn client(
    headers: &HeaderMap,
    peer: IpAddr,
    secure: bool,
    forwarded: Option<NonZeroU8>,
) -> IpAddr {
    let peer = peer.to_canonical();
    let skipped = match forwarded {
        _ if secure => return peer,
        Some(added) => usize::from(added.get()).saturating_sub(1),
        None if is_loopback(peer) || is_inside(peer) => 0,
        None => return peer,
    };
    // Read from the end, as bytes: what a client put before the proxies' addresses may be
    // anything, as long as a header may be, and must cost glave nothing nor hide what they
    // added after it.
    headers
        .get_all(FORWARDED_FOR)
        .iter()
        .rev()
        .flat_map(|value| value.as_bytes().rsplit(|byte| *byte == b','))
        .nth(skipped)
        .and_then(|named| address(String::from_utf8_lossy(named).trim()))
        .map_or(peer, |named| named.to_canonical())
}

/// The address `named` is, written bare or with a port: `203.0.113.7`, `203.0.113.7:4321`,
/// `2001:db8::1` or `[2001:db8::1]:4321`.
fn address(named: &str) -> Option<IpAddr> {
    named
        .parse()
        .ok()
        .or_else(|| named.parse::<SocketAddr>().ok().map(|at| at.ip()))
}

/// The header a proxy names the address it took a connection from in, after any it was told.
const FORWARDED_FOR: &str = "x-forwarded-for";

/// Whether `host`, a `Host` header's name and any port, names this computer.
fn here(host: &str) -> bool {
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    };
    is_this_computer(name)
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
    fn a_request_comes_from_where_a_proxy_here_or_inside_says_and_else_from_its_peer() {
        let from = |peer: &str, forwarded: &[&str], secure: bool| {
            let mut headers = HeaderMap::new();
            for value in forwarded {
                headers.append(FORWARDED_FOR, HeaderValue::from_str(value).unwrap());
            }
            client(&headers, peer.parse().unwrap(), secure, None).to_string()
        };
        // A tunnel here, or a proxy on a private network: the address it took it from, last.
        assert_eq!(from("127.0.0.1", &["203.0.113.7"], false), "203.0.113.7");
        assert_eq!(
            from("172.18.0.3", &["198.51.100.1, 203.0.113.7"], false),
            "203.0.113.7",
            "what the client said first is not believed"
        );
        assert_eq!(
            from("::1", &["10.0.0.1", "[2001:db8::7]:4321"], false),
            "2001:db8::7"
        );
        assert_eq!(
            from("::ffff:127.0.0.1", &["203.0.113.7:80"], false),
            "203.0.113.7"
        );
        // Named nowhere, or nothing it can read: the proxy itself.
        assert_eq!(from("127.0.0.1", &[], false), "127.0.0.1");
        assert_eq!(from("10.1.2.3", &["unknown"], false), "10.1.2.3");
        // Glave's own HTTPS, or a peer on the internet: the peer, whatever it says.
        assert_eq!(from("127.0.0.1", &["203.0.113.7"], true), "127.0.0.1");
        assert_eq!(from("203.0.113.9", &["10.0.0.1"], false), "203.0.113.9");
    }

    #[test]
    fn behind_google_s_load_balancer_the_client_is_the_one_it_names_whatever_came_before() {
        // Google Cloud's external Application Load Balancer: `[<supplied>,]<client>,<itself>`,
        // from a peer of its own on the internet.
        let behind = |forwarded: &[&[u8]]| {
            let mut headers = HeaderMap::new();
            for value in forwarded {
                headers.append(FORWARDED_FOR, HeaderValue::from_bytes(value).unwrap());
            }
            let peer = "35.191.0.9".parse().unwrap();
            client(&headers, peer, false, NonZeroU8::new(2)).to_string()
        };
        assert_eq!(behind(&[b"203.0.113.7,34.120.1.1"]), "203.0.113.7");
        assert_eq!(
            behind(&[b"198.51.100.1, 10.0.0.1,203.0.113.7,34.120.1.1"]),
            "203.0.113.7",
            "what the visitor said first is passed over"
        );
        assert_eq!(
            behind(&[b"not an address \xff\xfe, ,203.0.113.7,34.120.1.1"]),
            "203.0.113.7",
            "nor does anything it put there lose the client"
        );
        assert_eq!(
            behind(&[b"198.51.100.1", b"203.0.113.7,34.120.1.1"]),
            "203.0.113.7",
            "a header sent twice reads as one list"
        );
        assert_eq!(
            behind(&[b"203.0.113.7"]),
            "35.191.0.9",
            "not through it: its peer"
        );
        assert_eq!(behind(&[]), "35.191.0.9");
        assert_eq!(
            behind(&[b"203.0.113.7, 2001:db8::7,34.120.1.1"]),
            "2001:db8::7"
        );
    }

    #[test]
    fn a_count_of_proxies_is_believed_from_any_peer_but_not_on_glave_s_own_https() {
        let from = |peer: &str, added: u8, secure: bool| {
            let mut headers = HeaderMap::new();
            let named = HeaderValue::from_static("198.51.100.1, 203.0.113.7, 34.120.1.1");
            headers.append(FORWARDED_FOR, named);
            client(
                &headers,
                peer.parse().unwrap(),
                secure,
                NonZeroU8::new(added),
            )
            .to_string()
        };
        // One proxy with a peer on the internet, as Cloudflare's: the last address it names.
        assert_eq!(from("104.16.0.9", 1, false), "34.120.1.1");
        // The count, not where the peer is, says which: here too.
        assert_eq!(from("127.0.0.1", 2, false), "203.0.113.7");
        assert_eq!(from("10.0.0.2", 3, false), "198.51.100.1");
        // Reached straight, its certificate won at its own port: no proxy to count.
        assert_eq!(from("203.0.113.9", 2, true), "203.0.113.9");
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
    fn a_request_comes_in_here_through_the_public_origin_or_from_elsewhere() {
        let at = |text: &str| text.parse::<IpAddr>().unwrap();
        let (local, remote) = (at("127.0.0.1"), at("203.0.113.9"));
        let origin = PublicOrigin::parse("https://glave.example.com").unwrap();
        let tunnel = ListenerNames::of("127.0.0.1:4180".parse().unwrap(), Some(&origin));
        let named = |host| headers(Some(host), None);
        let came = |names: Option<&ListenerNames>, host, peer| door(names, &named(host), peer);
        assert_eq!(came(tunnel.as_ref(), "127.0.0.1:4180", local), Door::Here);
        assert_eq!(
            came(tunnel.as_ref(), "glave.example.com", local),
            Door::Public
        );
        let beyond = ListenerNames::of("0.0.0.0:443".parse().unwrap(), Some(&origin));
        assert_eq!(
            came(beyond.as_ref(), "glave.example.com", remote),
            Door::Public
        );
        assert_eq!(
            came(beyond.as_ref(), "glave.example.com", local),
            Door::Public
        );
        assert_eq!(came(beyond.as_ref(), "localhost:443", local), Door::Here);
        assert_eq!(came(beyond.as_ref(), "localhost:443", remote), Door::Public);
        assert_eq!(came(None, "10.0.0.5:8080", remote), Door::Elsewhere);
        assert_eq!(came(None, "127.0.0.1:8080", local), Door::Here);
    }

    #[test]
    fn ipv6_loopback_has_its_names_too() {
        let ipv6 = ListenerNames::of("[::1]:4180".parse().unwrap(), None).unwrap();
        assert!(ipv6.allow(&headers(Some("[::1]:4180"), Some("http://[::1]:4180"))));
        // A browser at localhost that reaches this address first.
        assert!(ipv6.allow(&headers(
            Some("localhost:4180"),
            Some("http://localhost:4180")
        )));
        for (host, page) in [
            (Some("localhost:4180"), Some("http://localhost:4181")),
            (Some("localhost:4181"), None),
            (Some("[::1]:4180"), Some("http://localhost:4180")),
        ] {
            assert!(!ipv6.allow(&headers(host, page)), "{host:?} {page:?}");
        }
    }
}
