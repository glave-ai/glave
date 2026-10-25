//! Where clouds serve a machine's credentials: the one list of those addresses, which every
//! part of glave that decides where the agent's connections go refuses.

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

    /// Whether `ip` is in this block, as written: an IPv4 address written as IPv6
    /// (`::ffff:a.b.c.d`) is not in an IPv4 block.
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
/// the agent where glave decides where its connections go: workers' network policy on
/// Kubernetes and `web_fetch`; not yet from commands in this machine's sandbox, whose rules
/// cannot name an address. glave's own process still reaches them, for its own credentials
/// (`specs/sandbox.md`, "Cloud metadata"). A single address stays single where the block
/// around it is someone else's.
pub const CLOUD_METADATA: [Network; 9] = [
    // IPv4 link-local. The metadata service at 169.254.169.254 on AWS, Google Cloud, Azure,
    // Oracle Cloud, DigitalOcean and Akamai (Linode), and on Hetzner, as cloud-init's Hetzner
    // source reads it; Scaleway's at 169.254.42.42; AWS's container credentials (ECS) at
    // 169.254.170.2, as `glave-aws` reaches them, and pod credentials (EKS Pod Identity) at
    // 169.254.170.23. Tencent Cloud documents only a name, metadata.tencentyun.com, reported
    // at 169.254.0.23.
    // docs.aws.amazon.com/AWSEC2/latest/UserGuide/instancedata-data-retrieval.html,
    // docs.aws.amazon.com/eks/latest/userguide/pod-id-how-it-works.html,
    // docs.cloud.google.com/compute/docs/metadata/querying-metadata,
    // learn.microsoft.com/en-us/azure/virtual-machines/instance-metadata-service,
    // docs.oracle.com/en-us/iaas/Content/Compute/Tasks/gettingmetadata.htm,
    // docs.digitalocean.com/products/droplets/how-to/retrieve-droplet-metadata/,
    // techdocs.akamai.com/cloud-computing/docs/access-the-metadata-service-api,
    // docs.cloud-init.io/en/latest/reference/datasources/scaleway.html,
    // intl.cloud.tencent.com/document/product/213/4934
    Network::v4(Ipv4Addr::new(169, 254, 0, 0), 16),
    // Alibaba Cloud's metadata service, RAM role credentials included, inside carrier NAT
    // (100.64.0.0/10), which other networks use: the one address.
    // alibabacloud.com/help/en/ecs/user-guide/manage-instance-metadata
    Network::v4(Ipv4Addr::new(100, 100, 100, 200), 32),
    // Azure's WireServer, the VM agent's channel, through which anyone on the machine could
    // read VM extensions' protected settings (Mandiant's "WireServing", 2024); also Azure's
    // DNS, which workers reach on port 53 anyway.
    // learn.microsoft.com/en-us/azure/virtual-network/what-is-ip-address-168-63-129-16
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
    // Akamai's (Linode's) metadata service over IPv6.
    // techdocs.akamai.com/cloud-computing/docs/access-the-metadata-service-api
    Network::v6(
        Ipv6Addr::new(0xfd00, 0xa9fe, 0xa9fe, 0, 0, 0, 0, 0x0001),
        128,
    ),
    // Scaleway's metadata service over IPv6.
    // docs.cloud-init.io/en/latest/reference/datasources/scaleway.html
    Network::v6(Ipv6Addr::new(0xfd00, 0x0042, 0, 0, 0, 0, 0, 0x0042), 128),
];

/// Whether `ip` is where a cloud serves its machines' credentials ([`CLOUD_METADATA`]). An
/// IPv4 address written as IPv6 (`::ffff:a.b.c.d`), which a dual-stack socket reaches as
/// that IPv4 address, counts as the address it carries.
pub fn is_cloud_metadata(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    CLOUD_METADATA.iter().any(|network| network.contains(ip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_metadata_is_link_local_and_each_cloud_s_own_address_and_nothing_beside_them() {
        for metadata in [
            "169.254.169.254",
            "169.254.170.2",
            "169.254.170.23",
            "169.254.42.42",
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
            "fd00:a9fe:a9fe::1",
            "fd00:42::42",
            // Mapped, as a dual-stack socket reaches it.
            "::ffff:169.254.169.254",
            "::ffff:168.63.129.16",
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
            "fd00:42::43",
            "::1",
            "::ffff:8.8.8.8",
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
                "fd20:ce::254/128",
                "fd00:a9fe:a9fe::1/128",
                "fd00:42::42/128"
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
}
