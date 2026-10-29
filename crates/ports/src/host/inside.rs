//! The addresses that are not the internet: private and shared networks, where a cluster's
//! Pods, its servers and a company's own services live. Refused, beside the cloud metadata
//! addresses, wherever glave fences a computer off from everything but the internet: a
//! person's computer on Kubernetes (`specs/computers.md`, decision 12).

use std::net::{Ipv4Addr, Ipv6Addr};

use super::metadata::Network;

/// Private and reserved networks (RFC 1918 and 6598 for IPv4, RFC 4193 for IPv6), the
/// unspecified and the benchmarking blocks, and the reserved class E, which some clusters
/// give their Pods.
pub const INSIDE: [Network; 8] = [
    Network::v4(Ipv4Addr::UNSPECIFIED, 8),
    Network::v4(Ipv4Addr::new(10, 0, 0, 0), 8),
    Network::v4(Ipv4Addr::new(100, 64, 0, 0), 10),
    Network::v4(Ipv4Addr::new(172, 16, 0, 0), 12),
    Network::v4(Ipv4Addr::new(192, 168, 0, 0), 16),
    Network::v4(Ipv4Addr::new(198, 18, 0, 0), 15),
    Network::v4(Ipv4Addr::new(240, 0, 0, 0), 4),
    Network::v6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0), 7),
];
