//! Which addresses glave's own clients may reach where someone else chose the address, as
//! `web_fetch` and a remote MCP server: public internet addresses; private ones (this computer,
//! private networks, link-local, carrier NAT, multicast, reserved and documentation ranges),
//! where a request could read what only the person's own network should see, only where they
//! are allowed; and cloud metadata, where this machine's cloud serves its credentials (glave's
//! one list, [`CLOUD_METADATA`](super::CLOUD_METADATA)), never.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::metadata::is_cloud_metadata;

/// Whether a client may connect to `ip`: never to cloud metadata, which no setting lifts,
/// in whatever IPv6 form it is carried; otherwise a public address, or any where private
/// addresses are allowed (`private_ok`).
pub fn reachable(ip: IpAddr, private_ok: bool) -> bool {
    let carried = match ip {
        IpAddr::V6(v6) => embedded_v4(v6).map(IpAddr::V4),
        IpAddr::V4(_) => None,
    };
    let metadata = is_cloud_metadata(ip) || carried.is_some_and(is_cloud_metadata);
    !metadata && (private_ok || is_public(ip))
}

/// Whether `ip` is a public internet address. IPv6 forms that carry an IPv4 address (mapped,
/// compatible, NAT64 and 6to4) are judged by that address.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let private = ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0 // "this network"
        || (a == 100 && (64..128).contains(&b)) // carrier-grade NAT
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 192 && b == 88 && c == 99) // 6to4 relay anycast
        || (a == 198 && (18..20).contains(&b)) // benchmarking
        || a >= 240; // reserved
    !private
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let first = ip.segments().first().copied().unwrap_or(0);
    let private = ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00 // unique local, fc00::/7
        || (first & 0xffc0) == 0xfe80 // link-local, fe80::/10
        || (first & 0xffc0) == 0xfec0 // site-local (deprecated), fec0::/10
        || ip.segments().get(..2) == Some(&[0x2001, 0x0db8]) // documentation
        || (first & 0xe000) != 0x2000; // outside global unicast, 2000::/3
    !private
}

/// The IPv4 address an IPv6 address carries, in the forms that reach it: mapped
/// (`::ffff:a.b.c.d`), compatible (`::a.b.c.d`), NAT64 (`64:ff9b::a.b.c.d`) and 6to4
/// (`2002:aabb:ccdd::`).
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return Some(v4);
    }
    let (high, low) = match ip.segments() {
        [0, 0, 0, 0, 0, 0, high, low] if !(high == 0 && low <= 1) => (high, low),
        [0x64, 0xff9b, 0, 0, 0, 0, high, low] | [0x2002, high, low, ..] => (high, low),
        _ => return None,
    };
    let bits = (u32::from(high) << 16) | u32::from(low);
    Some(Ipv4Addr::from(bits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(text: &str) -> bool {
        is_public(text.parse().unwrap())
    }

    #[test]
    fn public_addresses_pass() {
        for ip in [
            "93.184.216.34",
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700::1111",
            "2a00:1450::1",
        ] {
            assert!(public(ip), "{ip}");
        }
    }

    #[test]
    fn private_v4_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "127.1.2.3",
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.255",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "192.0.0.8",
        ] {
            assert!(!public(ip), "{ip}");
        }
    }

    #[test]
    fn cloud_metadata_is_never_reached_even_where_private_addresses_are() {
        let reach = |text: &str, private_ok| reachable(text.parse().unwrap(), private_ok);
        for metadata in [
            "169.254.169.254",
            "100.100.100.200",
            "168.63.129.16",
            "fd00:ec2::254",
            "fe80::1",
            "::ffff:169.254.169.254",
            "::169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::",
            "2002:a83f:8110::",
        ] {
            assert!(!reach(metadata, true), "{metadata}");
            assert!(!reach(metadata, false), "{metadata}");
        }
        for private in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "fc00::1",
            "100.64.0.1",
        ] {
            assert!(reach(private, true), "{private}");
            assert!(!reach(private, false), "{private}");
        }
        assert!(reach("93.184.216.34", false));
        assert!(reach("2606:4700::1111", true));
    }

    #[test]
    fn private_v6_addresses_and_v4_inside_v6_are_refused() {
        for ip in [
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd00:ec2::254",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "64:ff9b::a00:1",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::",
        ] {
            assert!(!public(ip), "{ip}");
        }
        assert!(public("::ffff:8.8.8.8"));
        assert!(public("64:ff9b::808:808"));
    }
}
