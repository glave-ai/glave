//! The fence as rtnetlink requests, as `ip rule` and `ip route` would send them, each
//! acknowledged by the kernel before the next: rules that look DNS up in the main table and
//! everything else in glave's table first, then each block of the list refused there.

use std::io;
use std::net::IpAddr;

use glave_ports::host::{CLOUD_METADATA, Network};

/// glave's own routing table ("GL"), apart from the namespace's.
const TABLE: u32 = 0x474c;
/// The namespace's own table, where its routes out are.
const MAIN: u32 = 254;
/// The first of the fence's rules, all before the main table's own rule at 32766.
const PRIORITY: u32 = 100;
/// DNS, which stays open to every address, as workers' network policy keeps it on Kubernetes.
const DNS: u16 = 53;
/// How long the kernel has to acknowledge a request: it answers at once.
#[cfg(target_os = "linux")]
const ACKNOWLEDGED_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);

const RTM_NEWROUTE: u16 = 24;
const RTM_NEWRULE: u16 = 32;
const NLMSG_ERROR: u16 = 2;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_ACK: u16 = 0x4;
const NLM_F_EXCL: u16 = 0x200;
const NLM_F_CREATE: u16 = 0x400;
const RTA_DST: u16 = 1;
const RTA_TABLE: u16 = 15;
const RTPROT_STATIC: u8 = 4;
const RTN_PROHIBIT: u8 = 10;
const FR_ACT_TO_TBL: u8 = 1;
const FRA_PRIORITY: u16 = 6;
const FRA_TABLE: u16 = 15;
const FRA_IP_PROTO: u16 = 22;
const FRA_DPORT_RANGE: u16 = 24;
const IPPROTO_TCP: u8 = 6;
const IPPROTO_UDP: u8 = 17;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;

/// One request: what it makes sure of, in words, and its message.
struct Request {
    what: String,
    message: Vec<u8>,
}

/// Refuses the list in this namespace: IPv4, then IPv6 unless the namespace has none.
#[cfg(target_os = "linux")]
pub(super) fn fence() -> io::Result<()> {
    use rustix::net::netlink::SocketAddrNetlink;
    use rustix::net::sockopt::{Timeout, set_socket_timeout};
    use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType};

    let socket = rustix::net::socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        None,
    )?;
    set_socket_timeout(&socket, Timeout::Recv, Some(ACKNOWLEDGED_WITHIN))?;
    let kernel = SocketAddrNetlink::new(0, 0);
    for v6 in [false, true] {
        for request in requests(v6) {
            rustix::net::sendto(&socket, &request.message, SendFlags::empty(), &kernel)?;
            let mut answer = [0_u8; 1024];
            let (got, _) = rustix::net::recv(&socket, answer.as_mut_slice(), RecvFlags::empty())?;
            match acknowledged(answer.get(..got).unwrap_or_default()) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                // With IPv6 off in the namespace, nothing in it reaches an IPv6 address.
                Err(_) if v6 && without_ipv6() => break,
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("could not make sure {}: {error}", request.what),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Whether this namespace has no IPv6: none in the kernel, or turned off for every interface,
/// which only `NET_ADMIN` could turn back on.
#[cfg(target_os = "linux")]
fn without_ipv6() -> bool {
    std::fs::read_to_string("/proc/sys/net/ipv6/conf/all/disable_ipv6")
        .map_or(true, |off| off.trim() == "1")
}

/// The requests for IPv4 or IPv6: DNS over UDP and over TCP looked up in the main table,
/// everything else in glave's table first, and each block of the family refused there.
fn requests(v6: bool) -> Vec<Request> {
    let family = if v6 { AF_INET6 } else { AF_INET };
    let mut requests = vec![
        Request {
            what: "DNS over UDP still goes".into(),
            message: rule(family, PRIORITY, MAIN, Some(IPPROTO_UDP)),
        },
        Request {
            what: "DNS over TCP still goes".into(),
            message: rule(family, PRIORITY + 1, MAIN, Some(IPPROTO_TCP)),
        },
        Request {
            what: "the fence is looked in".into(),
            message: rule(family, PRIORITY + 2, TABLE, None),
        },
    ];
    requests.extend(
        CLOUD_METADATA
            .iter()
            .filter(|network| network.is_ipv4() != v6)
            .map(|network| Request {
                what: format!("{network} is refused"),
                message: route(*network),
            }),
    );
    requests
}

/// A request to make something new, failing if it is there: `body` after the header.
fn message(kind: u16, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len().saturating_add(16)).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(body.len().saturating_add(16));
    out.extend(length.to_ne_bytes());
    out.extend(kind.to_ne_bytes());
    out.extend((NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL).to_ne_bytes());
    // The sequence number and the sender's port id, which the kernel fills in.
    out.extend([0_u8; 8]);
    out.extend(body);
    out
}

/// Appends the attribute `kind` holding `data`, padded to four bytes.
fn attribute(out: &mut Vec<u8>, kind: u16, data: &[u8]) {
    let length = u16::try_from(data.len().saturating_add(4)).unwrap_or(u16::MAX);
    out.extend(length.to_ne_bytes());
    out.extend(kind.to_ne_bytes());
    out.extend(data);
    out.resize(out.len().next_multiple_of(4), 0);
}

/// `ip route add prohibit <network> table TABLE`.
fn route(network: Network) -> Vec<u8> {
    let (family, address) = match network.address() {
        IpAddr::V4(address) => (AF_INET, address.octets().to_vec()),
        IpAddr::V6(address) => (AF_INET6, address.octets().to_vec()),
    };
    // rtmsg: family, destination and source lengths, tos, table (given as an attribute),
    // protocol, scope (anywhere), type, flags.
    let mut body = vec![
        family,
        network.prefix(),
        0,
        0,
        0,
        RTPROT_STATIC,
        0,
        RTN_PROHIBIT,
    ];
    body.extend(0_u32.to_ne_bytes());
    attribute(&mut body, RTA_DST, &address);
    attribute(&mut body, RTA_TABLE, &TABLE.to_ne_bytes());
    message(RTM_NEWROUTE, &body)
}

/// `ip rule add priority <priority> [ipproto <dns> dport 53] lookup <table>`.
fn rule(family: u8, priority: u32, table: u32, dns: Option<u8>) -> Vec<u8> {
    // fib_rule_hdr: family, destination and source lengths, tos, table (given as an
    // attribute), two reserved, action, flags.
    let mut body = vec![family, 0, 0, 0, 0, 0, 0, FR_ACT_TO_TBL];
    body.extend(0_u32.to_ne_bytes());
    attribute(&mut body, FRA_PRIORITY, &priority.to_ne_bytes());
    attribute(&mut body, FRA_TABLE, &table.to_ne_bytes());
    if let Some(protocol) = dns {
        attribute(&mut body, FRA_IP_PROTO, &[protocol]);
        let ports: Vec<u8> = DNS
            .to_ne_bytes()
            .into_iter()
            .chain(DNS.to_ne_bytes())
            .collect();
        attribute(&mut body, FRA_DPORT_RANGE, &ports);
    }
    message(RTM_NEWRULE, &body)
}

/// Whether the kernel's `answer` acknowledges the request, or the error it gave.
fn acknowledged(answer: &[u8]) -> io::Result<()> {
    let kind = answer
        .get(4..6)
        .and_then(|bytes| <[u8; 2]>::try_from(bytes).ok())
        .map(u16::from_ne_bytes);
    let error = answer
        .get(16..20)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(i32::from_ne_bytes);
    match (kind, error) {
        (Some(NLMSG_ERROR), Some(0)) => Ok(()),
        (Some(NLMSG_ERROR), Some(errno)) => {
            Err(io::Error::from_raw_os_error(errno.saturating_neg()))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the kernel answered with something other than an acknowledgement",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The attributes after a message's header and its fixed part, as (kind, data).
    fn attributes(message: &[u8]) -> Vec<(u16, Vec<u8>)> {
        let mut rest = message.get(28..).unwrap_or_default();
        let mut found = Vec::new();
        while let (Some(length), Some(kind)) = (rest.get(0..2), rest.get(2..4)) {
            let length = usize::from(u16::from_ne_bytes(length.try_into().unwrap()));
            let kind = u16::from_ne_bytes(kind.try_into().unwrap());
            found.push((kind, rest.get(4..length).unwrap().to_vec()));
            rest = rest.get(length.next_multiple_of(4)..).unwrap_or_default();
        }
        found
    }

    #[test]
    fn each_block_of_the_list_is_one_prohibit_route_in_glave_s_table() {
        let routes: Vec<Request> = [false, true]
            .into_iter()
            .flat_map(requests)
            .filter(|request| request.message.get(4..6) == Some(&RTM_NEWROUTE.to_ne_bytes()[..]))
            .collect();
        let said: Vec<String> = routes.iter().map(|route| route.what.clone()).collect();
        assert_eq!(said.first().unwrap(), "169.254.0.0/16 is refused");
        let routes: Vec<Vec<u8>> = routes.into_iter().map(|route| route.message).collect();
        assert_eq!(routes.len(), CLOUD_METADATA.len());
        for (message, network) in routes.iter().zip(CLOUD_METADATA) {
            let length = u32::from_ne_bytes(message.get(0..4).unwrap().try_into().unwrap());
            assert_eq!(usize::try_from(length).unwrap(), message.len());
            assert_eq!(message.len() % 4, 0, "{network}");
            let family = if network.is_ipv4() { AF_INET } else { AF_INET6 };
            assert_eq!(
                message.get(16..24).unwrap(),
                [
                    family,
                    network.prefix(),
                    0,
                    0,
                    0,
                    RTPROT_STATIC,
                    0,
                    RTN_PROHIBIT
                ]
            );
            let address = match network.address() {
                IpAddr::V4(address) => address.octets().to_vec(),
                IpAddr::V6(address) => address.octets().to_vec(),
            };
            assert_eq!(
                attributes(message),
                [
                    (RTA_DST, address),
                    (RTA_TABLE, TABLE.to_ne_bytes().to_vec())
                ]
            );
        }
    }

    #[test]
    fn dns_is_looked_up_in_the_main_table_before_the_fence_and_everything_else_after() {
        for v6 in [false, true] {
            let rules: Vec<Vec<(u16, Vec<u8>)>> = requests(v6)
                .into_iter()
                .take(3)
                .map(|request| attributes(&request.message))
                .collect();
            let port: Vec<u8> = [DNS.to_ne_bytes(), DNS.to_ne_bytes()].concat();
            let priority = |n: u32| (FRA_PRIORITY, n.to_ne_bytes().to_vec());
            let table = |n: u32| (FRA_TABLE, n.to_ne_bytes().to_vec());
            assert_eq!(
                rules,
                [
                    vec![
                        priority(100),
                        table(MAIN),
                        (FRA_IP_PROTO, vec![IPPROTO_UDP]),
                        (FRA_DPORT_RANGE, port.clone())
                    ],
                    vec![
                        priority(101),
                        table(MAIN),
                        (FRA_IP_PROTO, vec![IPPROTO_TCP]),
                        (FRA_DPORT_RANGE, port)
                    ],
                    vec![priority(102), table(TABLE)],
                ]
            );
        }
    }

    #[test]
    fn an_acknowledgement_is_success_and_an_error_says_which() {
        let answer = |errno: i32| {
            let mut answer = vec![36, 0, 0, 0];
            answer.extend(NLMSG_ERROR.to_ne_bytes());
            answer.extend([0_u8; 10]);
            answer.extend(errno.to_ne_bytes());
            answer
        };
        assert!(acknowledged(&answer(0)).is_ok());
        assert_eq!(
            acknowledged(&answer(-17)).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            acknowledged(&answer(-1)).unwrap_err().raw_os_error(),
            Some(1)
        );
        assert!(acknowledged(&[]).is_err());
    }
}
