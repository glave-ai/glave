//! Where the processes of this machine's commands in the background listen: each TCP address
//! this machine's own loopback reaches, a loopback address or every address, as it was bound
//! (`specs/computers.md`, "Background commands"). A command's processes are those in its
//! process group and everything they started, followed by parent as the sandbox's own session
//! on Linux leaves the group; a process that left on purpose (a daemon) is not followed, as
//! stopping one does not follow it. On Linux it is read from `/proc`; on macOS from `lsof`, the
//! system's own, by its path; elsewhere nothing is found.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(any(target_os = "linux", test))]
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
#[cfg(any(target_os = "macos", test))]
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use glave_ports::host::is_loopback;
use glave_process::Processes;

/// How long a look may take: a few local reads, or one run of `lsof`.
const LOOKING: Duration = Duration::from_secs(5);

/// Each address found, by port first, so a listing reads lowest port first.
type Found = BTreeMap<u32, BTreeSet<(u16, IpAddr)>>;

/// Where each of `groups`, a process group by its leader's id, listens, by port, then address;
/// a group listening nowhere is left out.
pub(super) async fn addresses(
    groups: &[u32],
    processes: &Processes,
) -> BTreeMap<u32, Vec<SocketAddr>> {
    if groups.is_empty() {
        return BTreeMap::new();
    }
    let found = tokio::time::timeout(LOOKING, found(groups, processes))
        .await
        .unwrap_or_default();
    found
        .into_iter()
        .map(|(group, at)| {
            let at = at.into_iter().map(|(port, ip)| SocketAddr::new(ip, port));
            (group, at.collect())
        })
        .collect()
}

#[cfg(target_os = "linux")]
async fn found(groups: &[u32], _: &Processes) -> Found {
    let groups = groups.to_vec();
    tokio::task::spawn_blocking(move || proc::found(&groups))
        .await
        .unwrap_or_default()
}

/// The only `lsof` trusted: one earlier on `PATH` could be planted.
#[cfg(target_os = "macos")]
const LSOF: &str = "/usr/sbin/lsof";

/// The most of what `lsof` prints that is read: a few dozen bytes a socket, for far more
/// sockets than a workspace's commands open. Past it, `lsof` is stopped and what it printed
/// first is what is found.
#[cfg(target_os = "macos")]
const MOST_PRINTED: usize = 256 * 1024;

#[cfg(target_os = "macos")]
async fn found(groups: &[u32], processes: &Processes) -> Found {
    let groups: Vec<String> = groups.iter().map(u32::to_string).collect();
    let mut lsof = glave_process::Program::new(LSOF);
    // Numbers, not names; the groups' TCP sockets that listen; each one's group, family and
    // address.
    lsof.args(["-nP", "-a", "-g"])
        .arg(groups.join(","))
        .args(["-iTCP", "-sTCP:LISTEN", "-F", "gtn"])
        .no_input();
    // It ends with 1 when it found nothing: what it printed says what it found either way.
    match processes
        .output_at_most(&mut lsof, LOOKING, MOST_PRINTED)
        .await
    {
        Ok(output) => in_lsof(&String::from_utf8_lossy(&output.stdout)),
        Err(error) => {
            tracing::debug!(%error, "could not look where commands listen");
            BTreeMap::new()
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn found(_: &[u32], _: &Processes) -> Found {
    BTreeMap::new()
}

/// Whether a socket listening at `address` is reached at this machine's loopback.
fn reached_here(address: IpAddr) -> bool {
    address.is_unspecified() || is_loopback(address)
}

/// What `lsof -F gtn` printed: each group's listening addresses, from its `g` lines and, after
/// them, each socket's family (`tIPv4`, `tIPv6`) and address (`n127.0.0.1:5173`,
/// `n[::1]:5173`, `n*:5173` for every address of its family).
#[cfg(any(target_os = "macos", test))]
fn in_lsof(printed: &str) -> Found {
    let mut found = Found::new();
    let (mut group, mut six) = (None, false);
    for line in printed.lines() {
        if let Some(id) = line.strip_prefix('g') {
            group = id.parse().ok();
        } else if let Some(family) = line.strip_prefix('t') {
            six = family == "IPv6";
        } else if let (Some(name), Some(group)) = (line.strip_prefix('n'), group)
            && let Some((host, port)) = name.rsplit_once(':')
            && let Ok(port) = port.parse::<u16>()
        {
            let every = if six {
                IpAddr::V6(Ipv6Addr::UNSPECIFIED)
            } else {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            };
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            let address = if host == "*" {
                Some(every)
            } else {
                bare.parse().ok()
            };
            if let Some(address) = address.filter(|address| reached_here(*address)) {
                found.entry(group).or_default().insert((port, address));
            }
        }
    }
    found
}

/// A process, as `/proc/<pid>/stat` names it: its parent and its group.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Process {
    parent: u32,
    group: u32,
}

/// The parent and group in what `/proc/<pid>/stat` holds: `pid (name) state ppid pgrp …`, the
/// name in parentheses that may itself hold any of them, so read from the last.
#[cfg(any(target_os = "linux", test))]
fn in_stat(stat: &str) -> Option<Process> {
    let (_, after) = stat.rsplit_once(')')?;
    let mut fields = after.split_whitespace().skip(1);
    let parent = fields.next()?.parse().ok()?;
    let group = fields.next()?.parse().ok()?;
    Some(Process { parent, group })
}

/// The deepest a command's processes are followed from its group, parent by parent.
#[cfg(any(target_os = "linux", test))]
const DEEPEST: usize = 64;

/// Which of `groups` each of `processes` belongs to: its own group, or the first group of a
/// process above it.
#[cfg(any(target_os = "linux", test))]
fn owners(processes: &HashMap<u32, Process>, groups: &[u32]) -> HashMap<u32, u32> {
    let groups: HashSet<u32> = groups.iter().copied().collect();
    let owner = |pid: u32| {
        let mut at = processes.get(&pid)?;
        for _ in 0..DEEPEST {
            if groups.contains(&at.group) {
                return Some(at.group);
            }
            at = processes.get(&at.parent)?;
        }
        None
    };
    processes
        .keys()
        .filter_map(|pid| owner(*pid).map(|group| (*pid, group)))
        .collect()
}

/// The sockets listening in a table of `/proc/net/tcp` or `tcp6`, each by its inode, with its
/// address, where this machine's loopback reaches it. An address is printed as the words of
/// its bytes in network order, each read as this machine reads a number; the port as a number.
#[cfg(any(target_os = "linux", test))]
fn in_table(table: &str) -> Vec<(u64, SocketAddr)> {
    /// A socket that listens, as the table writes its state.
    const LISTEN: &str = "0A";
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (local, state, inode) = (fields.get(1)?, fields.get(3)?, fields.get(9)?);
            if *state != LISTEN {
                return None;
            }
            let (address, port) = local.split_once(':')?;
            let address = in_hex(address)?;
            let port = u16::from_str_radix(port, 16).ok()?;
            let at = SocketAddr::new(address, port);
            reached_here(address).then_some((inode.parse().ok()?, at))
        })
        .collect()
}

/// An address as `/proc/net/tcp` writes it: one word in hex for IPv4, four for IPv6.
#[cfg(any(target_os = "linux", test))]
fn in_hex(hex: &str) -> Option<IpAddr> {
    let mut bytes = Vec::with_capacity(16);
    for word in 0..hex.len() / 8 {
        let digits = hex.get(word * 8..word * 8 + 8)?;
        bytes.extend(u32::from_str_radix(digits, 16).ok()?.to_ne_bytes());
    }
    match hex.len() {
        8 => <[u8; 4]>::try_from(bytes).ok().map(IpAddr::from),
        32 => <[u8; 16]>::try_from(bytes).ok().map(IpAddr::from),
        _ => None,
    }
}

/// The inode in a link of `/proc/<pid>/fd` to a socket: `socket:[12345]`.
#[cfg(any(target_os = "linux", test))]
fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// Reading `/proc`, on this machine's own network namespace: a sandboxed command's socket in
/// a namespace of its own (the sandbox without the network) is in none of its tables, and no
/// one here could reach it.
#[cfg(target_os = "linux")]
mod proc {
    use std::collections::HashMap;
    use std::fs;

    use super::{Found, Process, in_stat, in_table, owners, socket_inode};

    /// The most processes, and the most open files of each, looked at.
    const MOST: usize = 65_536;

    pub(super) fn found(groups: &[u32]) -> Found {
        let owned = owners(&processes(), groups);
        let mut sockets: HashMap<u64, u32> = HashMap::new();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "each socket is noted by its inode, whatever the order"
        )]
        for (pid, group) in &owned {
            let Ok(files) = fs::read_dir(format!("/proc/{pid}/fd")) else {
                continue;
            };
            for file in files.flatten().take(MOST) {
                let link = fs::read_link(file.path()).ok();
                if let Some(inode) = link
                    .as_ref()
                    .and_then(|link| socket_inode(&link.to_string_lossy()))
                {
                    sockets.insert(inode, *group);
                }
            }
        }
        let mut found = Found::new();
        for table in ["/proc/self/net/tcp", "/proc/self/net/tcp6"] {
            let Ok(text) = fs::read_to_string(table) else {
                continue;
            };
            for (inode, at) in in_table(&text) {
                if let Some(group) = sockets.get(&inode) {
                    found
                        .entry(*group)
                        .or_default()
                        .insert((at.port(), at.ip()));
                }
            }
        }
        found
    }

    /// Every process in sight, by its id.
    fn processes() -> HashMap<u32, Process> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return HashMap::new();
        };
        entries
            .flatten()
            .take(MOST)
            .filter_map(|entry| {
                let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
                let stat = fs::read_to_string(entry.path().join("stat")).ok()?;
                Some((pid, in_stat(&stat)?))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
