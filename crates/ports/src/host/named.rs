//! A host as a URL or a connection names it: a name or an address, read one way everywhere.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

/// A host a URL or a connection names: a name, kept in lower case with no final dot, or an
/// address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// A name, which glave looks up only once it is allowed.
    Name(String),
    /// An address, written as the host (`93.184.216.34`, `[2606:4700::1111]`).
    Address(IpAddr),
}

impl Host {
    /// The host `text` names: an IPv4 address, a bracketed IPv6 one, or a name of letters,
    /// digits, `-` and `_` in labels of 1 to 63 characters, 253 in all, not all of them digits
    /// (which a resolver would read as an address, `127.1`), in any case, with or without a
    /// final dot.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(v6) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            return v6.parse().ok().map(Self::Address);
        }
        if let Ok(address) = text.parse::<Ipv4Addr>() {
            return Some(Self::Address(IpAddr::V4(address)));
        }
        let name = text.strip_suffix('.').unwrap_or(text).to_ascii_lowercase();
        let labels_ok = name.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        });
        // A name that reads as an IPv4 address is one; it is not a name (`1.2.3.4.`).
        let numeric = name
            .split('.')
            .all(|label| label.chars().all(|c| c.is_ascii_digit()));
        (labels_ok && !numeric && name.len() <= 253).then_some(Self::Name(name))
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::Address(IpAddr::V6(v6)) => write!(f, "[{v6}]"),
            Self::Address(IpAddr::V4(v4)) => write!(f, "{v4}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(text: &str) -> Host {
        Host::parse(text).unwrap()
    }

    #[test]
    fn hosts_are_names_or_addresses() {
        assert_eq!(host("Example.COM."), Host::Name("example.com".into()));
        assert_eq!(
            host("93.184.216.34"),
            Host::Address("93.184.216.34".parse().unwrap())
        );
        assert_eq!(host("[::1]"), Host::Address("::1".parse().unwrap()));
        for bad in [
            "",
            ".",
            "a..b",
            "a b",
            "::1",
            "[nope]",
            "127.1",
            "2130706433",
            "1.2.3.4.",
        ] {
            assert_eq!(Host::parse(bad), None, "{bad}");
        }
        assert_eq!(Host::parse(&"a".repeat(64)), None);
        assert_eq!(host("[2606:4700::1111]").to_string(), "[2606:4700::1111]");
    }
}
