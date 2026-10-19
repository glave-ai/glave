//! Whether a host is this computer: the one check behind every "https, or plain http only
//! here" rule and every "only from this computer" door.

use std::net::IpAddr;

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

#[cfg(test)]
mod tests {
    use super::*;

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
