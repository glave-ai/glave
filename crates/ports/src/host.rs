//! Whether a host is this computer: the one check behind every "https, or plain http only
//! here" rule and every "only from this computer" door. Where clouds serve a machine's
//! credentials, the one list of those addresses. And origins: where a host is, as its scheme
//! and name, read one way for every origin glave keeps.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A block of addresses: those whose first `prefix` bits are `address`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Network {
    address: IpAddr,
    prefix: u8,
}

impl Network {
    const fn v4(address: Ipv4Addr, prefix: u8) -> Self {
        Self {
            address: IpAddr::V4(address),
            prefix,
        }
    }

    const fn v6(address: Ipv6Addr, prefix: u8) -> Self {
        Self {
            address: IpAddr::V6(address),
            prefix,
        }
    }

    /// Whether `ip` is in this block. An IPv4 address written as IPv6 (`::ffff:a.b.c.d`) is
    /// not: whoever reads one judges the address it carries.
    fn contains(self, ip: IpAddr) -> bool {
        match (self.address, ip) {
            (IpAddr::V4(block), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32_u32.saturating_sub(self.prefix.into()))
                    .unwrap_or(0);
                u32::from(block) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(block), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128_u32.saturating_sub(self.prefix.into()))
                    .unwrap_or(0);
                u128::from(block) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }

    /// Whether this is a block of IPv4 addresses.
    pub fn is_ipv4(self) -> bool {
        self.address.is_ipv4()
    }
}

impl fmt::Display for Network {
    /// In CIDR notation: `169.254.0.0/16`, `fe80::/10`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

/// Where clouds serve a machine's credentials and setup data to any process on it that asks,
/// checked on 2026-10-25 against each cloud's documents, the source beside each. Withheld from
/// the agent where glave decides where its connections go: workers' network policy and
/// `web_fetch`; not yet from commands in this machine's sandbox, whose rules cannot name an
/// address. glave's own process still reaches them, for its own credentials
/// (`specs/sandbox.md`, "Cloud metadata"). A single address stays single where the block
/// around it is someone else's.
pub const CLOUD_METADATA: [Network; 7] = [
    // IPv4 link-local. The metadata service at 169.254.169.254 on AWS, Google Cloud, Azure,
    // Oracle Cloud and DigitalOcean, and on Hetzner (which documents no address; cloud-init's
    // Hetzner source reads it there); AWS's container credentials (ECS) at 169.254.170.2, as
    // `glave-aws` reaches them, and pod credentials (EKS Pod Identity) at 169.254.170.23.
    // Tencent Cloud documents only a name, metadata.tencentyun.com, reported at 169.254.0.23.
    // docs.aws.amazon.com/AWSEC2/latest/UserGuide/instancedata-data-retrieval.html,
    // docs.aws.amazon.com/eks/latest/userguide/pod-id-how-it-works.html,
    // docs.cloud.google.com/compute/docs/metadata/querying-metadata,
    // learn.microsoft.com/en-us/azure/virtual-machines/instance-metadata-service,
    // docs.oracle.com/en-us/iaas/Content/Compute/Tasks/gettingmetadata.htm,
    // docs.digitalocean.com/products/droplets/how-to/retrieve-droplet-metadata/,
    // intl.cloud.tencent.com/document/product/213/4934
    Network::v4(Ipv4Addr::new(169, 254, 0, 0), 16),
    // Alibaba Cloud's metadata service, RAM role credentials included, inside carrier NAT
    // (100.64.0.0/10), which other networks use: the one address.
    // alibabacloud.com/help/en/ecs/user-guide/manage-instance-metadata
    Network::v4(Ipv4Addr::new(100, 100, 100, 200), 32),
    // Azure's WireServer, the VM agent's channel, through which anyone on the machine could
    // read VM extensions' protected settings (Mandiant's "WireServing", 2024); also Azure's
    // DNS, which workers reach on port 53 anyway. learn.microsoft.com/en-us/azure/virtual-network/what-is-ip-address-168-63-129-16
    Network::v4(Ipv4Addr::new(168, 63, 129, 16), 32),
    // IPv6 link-local.
    Network::v6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 0), 10),
    // AWS's metadata service over IPv6 (Nitro instances).
    // docs.aws.amazon.com/AWSEC2/latest/UserGuide/instancedata-data-retrieval.html
    Network::v6(Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254), 128),
    // EKS Pod Identity's agent over IPv6, as `glave-aws` reaches it.
    // docs.aws.amazon.com/eks/latest/userguide/pod-id-agent-setup.html
    Network::v6(Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0023), 128),
    // Google Cloud's metadata service on IPv6-only machines.
    // docs.cloud.google.com/compute/docs/metadata/querying-metadata
    Network::v6(Ipv6Addr::new(0xfd20, 0x00ce, 0, 0, 0, 0, 0, 0x0254), 128),
];

/// Whether `ip` is where a cloud serves its machines' credentials ([`CLOUD_METADATA`]).
pub fn is_cloud_metadata(ip: IpAddr) -> bool {
    CLOUD_METADATA.iter().any(|network| network.contains(ip))
}

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
/// tunnel or proxy in front of it. Never this computer's own name, nor an address.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicOrigin {
    origin: String,
}

impl PublicOrigin {
    /// The origin `text` names, in any case and with or without a final `/`; none when it is
    /// not an https origin of a name.
    pub fn parse(text: &str) -> Option<Self> {
        let origin = text.trim().trim_end_matches('/').to_ascii_lowercase();
        let (scheme, host) = self::origin(&origin)?;
        let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
        // An IPv6 address is bracketed; an IPv4 one parses.
        let address = name.starts_with('[') || name.parse::<IpAddr>().is_ok();
        let public = scheme == "https" && !address && !is_this_computer(name);
        public.then_some(Self { origin })
    }

    /// The origin itself, as a browser's `Origin` header names it: `https://glave.example.com`.
    pub fn as_str(&self) -> &str {
        &self.origin
    }

    /// Its host and any port, as a request's `Host` header names it: `glave.example.com`.
    pub fn authority(&self) -> &str {
        self.origin.strip_prefix("https://").unwrap_or(&self.origin)
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
        ] {
            assert!(PublicOrigin::parse(refused).is_none(), "{refused}");
        }
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
    fn cloud_metadata_is_link_local_and_each_cloud_s_own_address_and_nothing_beside_them() {
        for metadata in [
            "169.254.169.254",
            "169.254.170.2",
            "169.254.170.23",
            "169.254.0.23",
            "169.254.0.0",
            "169.254.255.255",
            "100.100.100.200",
            "168.63.129.16",
            "fe80::1",
            "fe80::a9fe:a9fe",
            "febf:ffff::1",
            "fd00:ec2::254",
            "fd00:ec2::23",
            "fd20:ce::254",
        ] {
            assert!(is_cloud_metadata(metadata.parse().unwrap()), "{metadata}");
        }
        for beside in [
            "169.253.255.255",
            "169.255.0.0",
            // Carrier NAT and Azure's other addresses are other people's.
            "100.100.100.201",
            "100.64.0.1",
            "168.63.129.17",
            "127.0.0.1",
            "10.0.0.1",
            "8.8.8.8",
            "fe7f:ffff::1",
            "fec0::1",
            "fd00:ec2::253",
            "fd00:ec2::255",
            "fd20:ce::253",
            "::1",
            // Mapped: whoever reads one judges the address it carries.
            "::ffff:169.254.169.254",
        ] {
            assert!(!is_cloud_metadata(beside.parse().unwrap()), "{beside}");
        }
        let written: Vec<String> = CLOUD_METADATA.iter().map(ToString::to_string).collect();
        assert_eq!(
            written,
            [
                "169.254.0.0/16",
                "100.100.100.200/32",
                "168.63.129.16/32",
                "fe80::/10",
                "fd00:ec2::254/128",
                "fd00:ec2::23/128",
                "fd20:ce::254/128"
            ]
        );
        let v4 = CLOUD_METADATA.iter().filter(|n| n.is_ipv4()).count();
        assert_eq!(v4, 3);
    }

    #[test]
    fn a_block_of_every_address_or_of_one_holds_what_it_says() {
        let all = Network::v4(Ipv4Addr::UNSPECIFIED, 0);
        assert!(all.contains("203.0.113.9".parse().unwrap()));
        assert!(!all.contains("2001:db8::1".parse().unwrap()));
        let one = Network::v4(Ipv4Addr::new(192, 0, 2, 1), 32);
        assert!(one.contains("192.0.2.1".parse().unwrap()));
        assert!(!one.contains("192.0.2.2".parse().unwrap()));
        assert!(Network::v6(Ipv6Addr::UNSPECIFIED, 0).contains("2001:db8::1".parse().unwrap()));
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
