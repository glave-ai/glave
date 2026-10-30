//! Whether a host is this computer: the one check behind every "https, or plain http only
//! here" rule and every "only from this computer" door. Where clouds serve a machine's
//! credentials, the one list of those addresses. And origins: where a host is, as its scheme
//! and name, read one way for every origin glave keeps.

mod inside;
mod metadata;

use std::fmt;
use std::net::IpAddr;

pub use inside::INSIDE;
pub use metadata::{CLOUD_METADATA, Network, is_cloud_metadata};

/// Whether `host`, as a URL or a connection names it, is this computer: `localhost` in any
/// case, or a loopback address, bare (`::1`) or bracketed (`[::1]`).
pub fn is_this_computer(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|inside| inside.strip_suffix(']'))
        .unwrap_or(host);
    host.eq_ignore_ascii_case("localhost") || bare.parse().is_ok_and(is_loopback)
}

/// Whether `address` is this computer: `127.0.0.0/8`, `::1`, or `127.0.0.0/8` mapped into
/// IPv6.
pub fn is_loopback(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map_or_else(|| v6.is_loopback(), |v4| v4.is_loopback()),
    }
}

/// The scheme and the host with any port of `url`, when it is an origin and nothing more:
/// `https://` and a host of letters, digits, `.` and `-` with any port, or `http://` to this
/// computer; no path, user name, query or fragment. `url` is read as given, so lower case it
/// and take any final `/` off first.
pub fn origin(url: &str) -> Option<(&str, &str)> {
    let (scheme, host) = url.split_once("://")?;
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !name.ends_with(':') => (name, Some(port)),
        _ => (host, None),
    };
    let named = name == "[::1]"
        || (!name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'));
    let ported = port.is_none_or(|port| {
        !port.is_empty() && port.len() <= 5 && port.chars().all(|c| c.is_ascii_digit())
    });
    let allowed = scheme == "https" || (scheme == "http" && is_this_computer(name));
    (named && ported && allowed).then_some((scheme, host))
}

/// Where people reach a server from elsewhere: `https://` and its name, with any port, and
/// nothing more, kept in lower case. Something serves HTTPS for it: the server itself, or a
/// tunnel or proxy in front of it. Never an address. Or, set so on purpose, this computer's
/// `http://localhost:<port>`, for people who share one computer, each in a browser of their
/// own: a name, since WebAuthn takes no address for its relying party, with its port, which
/// must be the one the server serves at.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicOrigin {
    origin: String,
}

impl PublicOrigin {
    /// The origin `text` names, in any case and with or without a final `/`; none when it is
    /// neither an https origin of a name nor `http://localhost` with a port.
    pub fn parse(text: &str) -> Option<Self> {
        let origin = text.trim().trim_end_matches('/').to_ascii_lowercase();
        let (scheme, host) = self::origin(&origin)?;
        let (name, port) = host
            .rsplit_once(':')
            .map_or((host, None), |(name, port)| (name, Some(port)));
        // An IPv6 address is bracketed; an IPv4 one parses.
        let address = name.starts_with('[') || name.parse::<IpAddr>().is_ok();
        let public = scheme == "https" && !address && !is_this_computer(name);
        let here = scheme == "http"
            && name == "localhost"
            && port.is_some_and(|port| port.parse::<u16>().is_ok_and(|port| port != 0));
        (public || here).then_some(Self { origin })
    }

    /// The origin itself, as a browser's `Origin` header names it: `https://glave.example.com`.
    pub fn as_str(&self) -> &str {
        &self.origin
    }

    /// Its host and any port, as a request's `Host` header names it: `glave.example.com`.
    pub fn authority(&self) -> &str {
        self.origin
            .split_once("://")
            .map_or(self.origin.as_str(), |(_, authority)| authority)
    }

    /// The port of `http://localhost:<port>`, when it is this computer's: where the server must
    /// serve, on loopback alone, for it to be its origin.
    pub fn here(&self) -> Option<u16> {
        let port = self.origin.strip_prefix("http://localhost:")?;
        port.parse().ok()
    }
}

impl fmt::Display for PublicOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.origin)
    }
}

impl fmt::Debug for PublicOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicOrigin({})", self.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_public_origin_is_https_and_a_name_and_nothing_more() {
        let origin = PublicOrigin::parse(" HTTPS://Glave.Example.com/ ").unwrap();
        assert_eq!(origin.as_str(), "https://glave.example.com");
        assert_eq!(origin.authority(), "glave.example.com");
        assert_eq!(origin.to_string(), "https://glave.example.com");
        let ported = PublicOrigin::parse("https://glave.example.com:8443").unwrap();
        assert_eq!(ported.authority(), "glave.example.com:8443");
        assert!(PublicOrigin::parse("https://box.tail1234.ts.net").is_some());
        for refused in [
            "",
            "glave.example.com",
            "http://glave.example.com",
            "https://glave.example.com/app",
            "https://glave.example.com?x=1",
            "https://glave.example.com#x",
            "https://me@glave.example.com",
            "https://glave.example.com:",
            "https://glave.example.com:123456",
            "https://localhost",
            "https://127.0.0.1",
            "https://[::1]",
            "https://203.0.113.5",
            "ftp://glave.example.com",
            "http://localhost",
            "http://127.0.0.1:4180",
            "http://[::1]:4180",
            "http://localhost:4180/app",
            "http://localhost:99999",
            "http://localhost:0",
        ] {
            assert!(PublicOrigin::parse(refused).is_none(), "{refused}");
        }
        assert_eq!(origin.here(), None);
    }

    #[test]
    fn this_computer_is_an_origin_only_as_localhost_and_its_port() {
        let here = PublicOrigin::parse("http://LOCALHOST:4180/").unwrap();
        assert_eq!(here.as_str(), "http://localhost:4180");
        assert_eq!(here.authority(), "localhost:4180");
        assert_eq!(here.here(), Some(4180));
    }

    #[test]
    fn an_origin_is_https_anywhere_or_http_here_and_nothing_more() {
        assert_eq!(
            origin("https://git.example.com:8443"),
            Some(("https", "git.example.com:8443"))
        );
        assert_eq!(
            origin("http://localhost:3000"),
            Some(("http", "localhost:3000"))
        );
        assert_eq!(origin("http://[::1]:3000"), Some(("http", "[::1]:3000")));
        for refused in [
            "http://git.example.com",
            "https://git.example.com/org",
            "https://",
            "git.example.com",
        ] {
            assert_eq!(origin(refused), None, "{refused}");
        }
    }

    #[test]
    fn loopback_is_this_computer_only() {
        for here in ["127.0.0.1", "127.1.2.3", "::1", "::ffff:127.0.0.1"] {
            assert!(is_loopback(here.parse().unwrap()), "{here}");
        }
        for there in [
            "10.0.0.1",
            "192.168.1.2",
            "::ffff:10.0.0.1",
            "2001:db8::1",
            "0.0.0.0",
        ] {
            assert!(!is_loopback(there.parse().unwrap()), "{there}");
        }
    }

    #[test]
    fn a_host_is_this_computer_by_name_or_loopback_address() {
        for here in [
            "localhost",
            "LocalHost",
            "127.0.0.1",
            "127.0.0.2",
            "::1",
            "[::1]",
        ] {
            assert!(is_this_computer(here), "{here}");
        }
        for there in [
            "",
            "example.com",
            "localhost.example.com",
            "10.0.0.1",
            "[::2]",
            "::1]",
        ] {
            assert!(!is_this_computer(there), "{there}");
        }
    }
}
